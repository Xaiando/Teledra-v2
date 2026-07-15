"""swarm_architect skill — ingests massive blueprints and spawns specialized sub-jobs for the swarm."""

from __future__ import annotations
import os
import json

def execute(job: dict, ctx: dict) -> dict:
    log = ctx.get("log", print)
    inp = job.get("input", "")
    
    if isinstance(inp, str):
        try:
            inp = json.loads(inp)
        except Exception:
            inp = {"task": inp, "dir": "workspace/games/duchmans_mine_rebuild"}

    blueprint = inp.get("task", "")
    base_dir = inp.get("dir", "workspace/games/duchmans_mine_rebuild")
    
    log(f"[swarm_architect] Ingesting massive blueprint ({len(blueprint)} chars)...")
    
    # Generate 5 specific sub-jobs for the Task Forces based on the blueprint
    children = []
    task_forces = [
        ("alpha_core", "Task Force Alpha (Core Engine & FSM): Responsible for coding the master game loop, the state transition manager, and the global clock/temperature tick system. This team must ensure flawless memory handoffs between the town, desert, and cave states, ensuring the RNG seed remains stable."),
        ("beta_ui", "Task Force Beta (UI & Survival Math): Tasked with rendering the bottom status panel and wiring it to the underlying health, food, and water floats. This team must program the complex environmental modifiers, ensuring that temperatures above 90 degrees exponentially increase thirst, while temperatures below 50 degrees increase hunger, and must implement the inventory payload limits (base constraints vs. burro multipliers)."),
        ("gamma_procgen", "Task Force Gamma (Procedural Generation): Dedicated entirely to the RNG algorithms. This team must populate the massive desert grid, ensuring the logical placement of the river, the 100+ cave entrances, the Weaver's Needle sprite, and must select one random cave coordinate to securely hold the LDM endgame package."),
        ("delta_combat", "Task Force Delta (Combat & AI): Must write the rigid, keyboard-bound combat controller, explicitly denying mouse-aiming. This team is responsible for the behavior trees governing Bandits, Native Americans, and Snakes, as well as scripting the subdual and bounty payout logic for the Jail."),
        ("epsilon_minigames", "Task Force Epsilon (Minigames & Logic): Responsible for the standalone systemic modules. This includes the Panning RNG yield calculator, the Cave mining hit-detection, the precise Spider-Rope avoidance geometry, and the complex 5-Card Draw Poker AI governing the Dapper Dan encounters.")
    ]
    
    for force_dir, force_task in task_forces:
        child_dir = os.path.join(base_dir, force_dir).replace("\\", "/")
        
        child_prompt = f"{blueprint}\n\nYOUR SPECIFIC DIRECTIVE:\n{force_task}\n\nCompile your specific subsystem. Return only the raw code file for your module."
        
        children.append({
            "skill": "code_forge",
            "input": json.dumps({
                "task": child_prompt,
                "dir": child_dir,
                "profile": "simulation",
                "quality": "standard" # standard to avoid massive verification loops on partial components
            })
        })
        
    log(f"[swarm_architect] Spawned {len(children)} task force children for parallel compilation.")
    
    return {
        "ok": True,
        "output": "Delegated 5 Task Force components to the swarm.",
        "notes": "Blueprint split successfully into Alpha through Epsilon.",
        "children": children
    }
