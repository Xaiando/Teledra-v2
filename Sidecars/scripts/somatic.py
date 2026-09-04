"""Sample face, hand, and shoulder posture telemetry from the local camera."""

import json
import os
import sys
import time


def response(ok, message, data):
    print(json.dumps({
        "ok": ok,
        "capability": "somatic",
        "message": message,
        "data": data,
    }))


def main():
    if os.environ.get("TELEDRA_DISABLE_CAMERA"):
        response(True, "Somatic camera is disabled", {
            "face_detected": False,
            "hands_detected": False,
            "shoulder_asymmetry": None,
            "source": "disabled",
            "error": "TELEDRA_DISABLE_CAMERA is set",
        })
        return 0

    try:
        import cv2
    except Exception as error:
        response(True, "Somatic camera packages are unavailable", {
            "face_detected": False,
            "hands_detected": False,
            "shoulder_asymmetry": None,
            "source": "unavailable",
            "error": f"OpenCV unavailable: {error}",
        })
        return 0

    camera_index = int(os.environ.get("TELEDRA_CAMERA_INDEX", "0"))
    capture = cv2.VideoCapture(camera_index, cv2.CAP_DSHOW if os.name == "nt" else 0)
    if not capture.isOpened() and os.name == "nt":
        capture.release()
        capture = cv2.VideoCapture(camera_index)
    if not capture.isOpened():
        response(True, "Somatic camera could not be opened", {
            "face_detected": False,
            "hands_detected": False,
            "shoulder_asymmetry": None,
            "source": f"camera:{camera_index}",
            "error": "camera unavailable or already in use",
        })
        return 0

    frame = None
    try:
        deadline = time.monotonic() + 1.5
        while time.monotonic() < deadline:
            valid, candidate = capture.read()
            if valid:
                frame = candidate
            if frame is not None and time.monotonic() + 0.25 >= deadline:
                break
    finally:
        capture.release()

    if frame is None:
        response(True, "Somatic camera returned no frame", {
            "face_detected": False,
            "hands_detected": False,
            "shoulder_asymmetry": None,
            "source": f"camera:{camera_index}",
            "error": "no camera frame",
        })
        return 0

    face_detected = False
    hands_detected = False
    shoulder_asymmetry = None
    source = "opencv-haar"
    error = None
    try:
        import mediapipe as mp

        rgb = cv2.cvtColor(frame, cv2.COLOR_BGR2RGB)
        with mp.solutions.holistic.Holistic(
            static_image_mode=True,
            model_complexity=0,
            min_detection_confidence=0.45,
        ) as holistic:
            result = holistic.process(rgb)
        face_detected = result.face_landmarks is not None
        hands_detected = (
            result.left_hand_landmarks is not None
            or result.right_hand_landmarks is not None
        )
        if result.pose_landmarks is not None:
            landmarks = result.pose_landmarks.landmark
            left = landmarks[11]
            right = landmarks[12]
            if min(left.visibility, right.visibility) >= 0.35:
                shoulder_asymmetry = round(abs(left.y - right.y), 4)
        source = "mediapipe-holistic"
    except Exception as media_error:
        try:
            grayscale = cv2.cvtColor(frame, cv2.COLOR_BGR2GRAY)
            cascade_path = os.path.join(
                cv2.data.haarcascades, "haarcascade_frontalface_default.xml"
            )
            detector = cv2.CascadeClassifier(cascade_path)
            face_detected = len(detector.detectMultiScale(grayscale, 1.1, 4)) > 0
            error = f"posture model unavailable: {media_error}"
        except Exception as fallback_error:
            error = f"camera analysis failed: {fallback_error}"

    response(True, "Somatic camera sampled", {
        "face_detected": face_detected,
        "hands_detected": hands_detected,
        "shoulder_asymmetry": shoulder_asymmetry,
        "source": source,
        "error": error,
        "width": int(frame.shape[1]),
        "height": int(frame.shape[0]),
    })
    return 0


if __name__ == "__main__":
    sys.exit(main())
