import sys
import os
import json
import time
import cv2
from pathlib import Path

# Resolve HealthTool root
healthtool_root = os.environ.get("TELEDRA_HEALTHTOOL_ROOT")
if healthtool_root:
    sys.path.insert(0, str(Path(healthtool_root).expanduser().resolve()))
else:
    print(json.dumps({"error": "TELEDRA_HEALTHTOOL_ROOT not set"}), flush=True)
    sys.exit(1)

try:
    from somatic_cortex import SomaticCortex
except ImportError as e:
    print(json.dumps({"error": f"Import error: {str(e)}"}), flush=True)
    sys.exit(1)

# Adaptive resource policy:
# - ACTIVE  (~2 fps): a face was seen within the last IDLE_AFTER_S seconds.
# - IDLE    (1 frame / 5 s): nobody present; mediapipe work drops ~10x.
# - PROBE   (1 cheap check / 30 s): no camera connected; near-zero CPU,
#   recovers automatically when a camera is plugged in.
ACTIVE_INTERVAL = 0.5
IDLE_INTERVAL = 5.0
PROBE_INTERVAL = 30.0
IDLE_AFTER_S = 60.0
MAX_READ_FAILURES = 10


def open_camera():
    cap = cv2.VideoCapture(0)
    if cap.isOpened():
        return cap
    cap.release()
    return None


def main():
    default_model_dir = Path(healthtool_root) / "Neuralook" / "models" if healthtool_root else ""
    model_dir_env = os.environ.get("TELEDRA_SOMATIC_MODEL_DIR")
    model_dir = str(Path(model_dir_env).expanduser().resolve()) if model_dir_env else str(default_model_dir.resolve())
    try:
        cortex = SomaticCortex(model_dir=model_dir)
    except Exception as e:
        print(json.dumps({"error": f"Failed to initialize SomaticCortex: {str(e)}"}), flush=True)
        sys.exit(1)

    cap = None
    last_face_time = 0.0
    read_failures = 0
    announced_ready = False
    announced_no_camera = False

    while True:
        # PROBE state: no camera. Check occasionally, sleep otherwise.
        if cap is None:
            cap = open_camera()
            if cap is None:
                if not announced_no_camera:
                    print(json.dumps({"error": "No camera connected; somatic telemetry idle"}), flush=True)
                    announced_no_camera = True
                time.sleep(PROBE_INTERVAL)
                continue
            read_failures = 0
            announced_no_camera = False
            if not announced_ready:
                print(json.dumps({"status": "ready"}), flush=True)
                announced_ready = True

        # Drain buffered frames so throttled reads see a current image,
        # then read one frame for analysis.
        for _ in range(3):
            cap.grab()
        ret, frame = cap.read()
        if not ret:
            read_failures += 1
            if read_failures >= MAX_READ_FAILURES:
                # Camera disconnected mid-run: release and fall back to PROBE
                # instead of busy-spinning the loop at full CPU.
                cap.release()
                cap = None
                print(json.dumps({"error": "Camera disconnected; somatic telemetry idle"}), flush=True)
                continue
            time.sleep(1.0)
            continue
        read_failures = 0

        has_face = False
        try:
            results = cortex.analyze_frame(frame)
            stats = cortex.get_kinematic_stats(results)

            face_landmarks = results.get("face")
            if face_landmarks and hasattr(face_landmarks, "face_landmarks") and face_landmarks.face_landmarks:
                has_face = len(face_landmarks.face_landmarks) > 0

            hand_landmarks = results.get("hands")
            has_hands = False
            if hand_landmarks and hasattr(hand_landmarks, "hand_landmarks") and hand_landmarks.hand_landmarks:
                has_hands = len(hand_landmarks.hand_landmarks) > 0

            payload = {
                "face_detected": has_face,
                "hands_detected": has_hands,
                "shoulder_asymmetry": stats.get("shoulder_asymmetry") if stats else None
            }
            print(json.dumps(payload), flush=True)
        except Exception as e:
            print(json.dumps({"error": f"Loop error: {str(e)}"}), flush=True)

        if has_face:
            last_face_time = time.time()

        # ACTIVE vs IDLE throttle: full analysis cadence only while someone
        # is (recently) present; otherwise drop to a slow heartbeat.
        if time.time() - last_face_time > IDLE_AFTER_S:
            time.sleep(IDLE_INTERVAL)
        else:
            time.sleep(ACTIVE_INTERVAL)


if __name__ == "__main__":
    main()
