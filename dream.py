import os
import json
import sqlite3
import urllib.request
import urllib.parse
from datetime import datetime

# Define paths
# Follow the court's model selection: without this the background dream cycle
# consolidates memory using a different model than the one actually running.
CONFIG_PATH = os.environ.get("TELEDRA_CONFIG", "config.json")
CHAT_LOGS_PATH = "knowledge/chat_logs.jsonl"
LEARNED_MEMORY_PATH = "knowledge/learned_memory.json"
SELF_REFLECTIONS_PATH = "knowledge/self_reflections.json"
DB_PATH = "knowledge/memory.db"

def init_db():
    os.makedirs("knowledge", exist_ok=True)
    conn = sqlite3.connect(DB_PATH)
    cursor = conn.cursor()
    # Create main table
    cursor.execute("""
    CREATE TABLE IF NOT EXISTS memories (
        id INTEGER PRIMARY KEY AUTOINCREMENT,
        content TEXT NOT NULL,
        category TEXT,
        timestamp DATETIME DEFAULT CURRENT_TIMESTAMP
    )
    """)
    # Create FTS5 virtual table for BM25 search
    cursor.execute("""
    CREATE VIRTUAL TABLE IF NOT EXISTS memories_fts USING fts5(
        content,
        content_id UNINDEXED
    )
    """)
    conn.commit()
    return conn

def insert_memory(conn, content, category="general"):
    cursor = conn.cursor()
    # Insert into main table
    cursor.execute(
        "INSERT INTO memories (content, category) VALUES (?, ?)",
        (content, category)
    )
    content_id = cursor.lastrowid
    # Insert into FTS5 virtual table
    cursor.execute(
        "INSERT INTO memories_fts (content, content_id) VALUES (?, ?)",
        (content, content_id)
    )
    conn.commit()

def load_config():
    if os.path.exists(CONFIG_PATH):
        with open(CONFIG_PATH, "r", encoding="utf-8") as f:
            return json.load(f)
    return {
        "api_url": "http://localhost:11434/v1/chat/completions",
        "model": "llama3"
    }

def call_llm(config, prompt):
    headers = {"Content-Type": "application/json"}
    payload = {
        "model": config.get("model", "llama3"),
        "messages": [
            {"role": "system", "content": "You are a precise, background cognitive curation assistant."},
            {"role": "user", "content": prompt}
        ],
        "temperature": 0.1,
        "response_format": {"type": "json_object"}
    }
    
    req = urllib.request.Request(
        config.get("api_url"),
        data=json.dumps(payload).encode("utf-8"),
        headers=headers,
        method="POST"
    )
    
    try:
        with urllib.request.urlopen(req, timeout=60) as resp:
            res_data = json.loads(resp.read().decode("utf-8"))
            return res_data["choices"][0]["message"]["content"]
    except Exception as e:
        print(f"Error calling LLM: {e}")
        return None

def process_dream():
    print("STATUS: Initializing Dreaming cycle...")
    if not os.path.exists(CHAT_LOGS_PATH) or os.path.getsize(CHAT_LOGS_PATH) == 0:
        print("No new chat logs to consolidate. dreaming complete.")
        return

    # Load logs
    logs = []
    with open(CHAT_LOGS_PATH, "r", encoding="utf-8-sig") as f:
        for line_num, line in enumerate(f, 1):
            if line.strip():
                try:
                    logs.append(json.loads(line.strip()))
                except Exception as e:
                    print(f"Warning: Failed to parse log line {line_num}: {e}")
                    # Skip corrupted or malformed lines gracefully instead of crashing the cycle

    print(f"Loaded {len(logs)} chat log messages.")

    # Cap the prompt size: long streams produce thousands of lines, which
    # overflow the local model's context and silently degrade extraction.
    if len(logs) > 400:
        logs = logs[-400:]
        print("Truncated to the most recent 400 messages for consolidation.")

    # Format log for prompt
    formatted_chat = ""
    for entry in logs:
        timestamp = entry.get("timestamp", "")
        sender = entry.get("sender", "")
        message = entry.get("message", "")
        formatted_chat += f"[{timestamp}] {sender}: {message}\n"

    prompt = f"""You are analyzing a chat log between the User and Teledra (a proud, sassy, transactional gothic monarch whose face is a natural porcelain-like visage -- it is NOT a mask -- wearing gothic armor and a golden halo).
    
    Your goal is to extract:
    1. "new_facts": Any new facts the User shared about themselves (e.g. name, preferences, life updates, interests, computer hardware, or likes/dislikes). Keep them concise and focused on the User.
    2. "reflections": Any self-reflections or critiques on Teledra's performance (e.g. if she broke character, talked too fast, was too friendly, violated lore restrictions, or made formatting/robotic mistakes). Write these as concise instructions for Teledra to avoid in the future (e.g. "Do not sound overly enthusiastic when greeting the user").

    Format your output strictly as a JSON object matching this structure:
    {{
        "new_facts": ["fact 1", "fact 2"],
        "reflections": ["critique 1", "critique 2"]
    }}

    Here is the chat log:
    {formatted_chat}
    """

    config = load_config()
    print("STATUS: Distilling memories and critiques via local LLM...")
    response_text = call_llm(config, prompt)
    if not response_text:
        print("Dreaming failed: LLM returned empty response.")
        return

    try:
        curated = json.loads(response_text)
    except Exception as e:
        print(f"Failed to parse LLM response as JSON: {e}\nRaw output: {response_text}")
        return

    new_facts = curated.get("new_facts", [])
    reflections = curated.get("reflections", [])

    conn = init_db()

    # 1. Update SQLite and learned_memory.json
    if new_facts:
        print(f"Distilled {len(new_facts)} new facts.")
        # Load active memory cache
        facts_cache = []
        if os.path.exists(LEARNED_MEMORY_PATH):
            try:
                with open(LEARNED_MEMORY_PATH, "r", encoding="utf-8") as f:
                    facts_cache = json.load(f)
            except:
                pass
        
        for fact in new_facts:
            # Save to SQLite
            insert_memory(conn, fact, "user_fact")
            print(f"Saved memory to DB: {fact}")
            
            # Save to active recall cache
            if fact not in facts_cache:
                facts_cache.append(fact)
        
        # Limit active recall cache to 10 entries to avoid context bloat
        if len(facts_cache) > 10:
            facts_cache = facts_cache[-10:]
            
        with open(LEARNED_MEMORY_PATH, "w", encoding="utf-8") as f:
            json.dump(facts_cache, f, indent=2)

    # 2. Update self_reflections.json
    if reflections:
        print(f"Distilled {len(reflections)} new self-reflections.")
        reflections_cache = []
        if os.path.exists(SELF_REFLECTIONS_PATH):
            try:
                with open(SELF_REFLECTIONS_PATH, "r", encoding="utf-8") as f:
                    reflections_cache = json.load(f)
            except:
                pass
                
        for ref in reflections:
            if ref not in reflections_cache:
                reflections_cache.append(ref)
                
        # Limit cache to 10 entries
        if len(reflections_cache) > 10:
            reflections_cache = reflections_cache[-10:]
            
        with open(SELF_REFLECTIONS_PATH, "w", encoding="utf-8") as f:
            json.dump(reflections_cache, f, indent=2)

    # 3. Consolidate and de-conflict memories
    consolidate_memories(conn, config)

    # 4. Archive/clear processed chat logs
    archive_path = f"knowledge/chat_logs_archive_{datetime.now().strftime('%Y%m%d_%H%M%S')}.jsonl"
    try:
        os.rename(CHAT_LOGS_PATH, archive_path)
        print(f"Archived recent chat logs to: {archive_path}")
    except Exception as e:
        print(f"Failed to archive chat logs: {e}")
        # fallback: clear the file
        open(CHAT_LOGS_PATH, "w").close()

    conn.close()
    print("STATUS: Dreaming cycle complete.")

def consolidate_memories(conn, config):
    print("STATUS: Consolidating and de-conflicting belief memory database...")
    cursor = conn.cursor()
    cursor.execute("SELECT id, content, timestamp FROM memories WHERE category='user_fact'")
    rows = cursor.fetchall()
    if not rows:
        return
        
    memories_list = ""
    for r in rows:
        memories_list += f"- [ID: {r[0]}, Date: {r[2]}] {r[1]}\n"
        
    prompt = f"""You are the Memory Consolidation Agent for Teledra.
    Your task is to audit her long-term SQLite database of facts and beliefs to resolve conflicts, remove duplicates, delete outdated noise, and update beliefs.
    
    Here is the list of current long-term memories:
    {memories_list}
    
    Instructions:
    1. Identify duplicate facts and merge them.
    2. Look for contradictions (e.g. if the user said they liked apples in 2026-06-04 but said they hate apples in 2026-06-05). Always resolve in favor of the more recent date.
    3. Prune trivial or temporary noise (e.g., extremely short-term states or random tangents).
    4. Produce a consolidated, clean list of active facts and core beliefs.
    
    Format your response strictly as a JSON object matching this structure:
    {{
        "consolidated_memories": [
            "Merged/updated fact 1",
            "Merged/updated fact 2"
        ],
        "deleted_ids": [1, 5, 8]  // The IDs of the old memories that are now merged, updated, or deleted as noise
    }}
    """
    
    response_text = call_llm(config, prompt)
    if not response_text:
        return
        
    try:
        data = json.loads(response_text)
        consolidated = data.get("consolidated_memories", [])
        deleted_ids = data.get("deleted_ids", [])
        
        if deleted_ids or consolidated:
            # Delete old rows
            for db_id in deleted_ids:
                cursor.execute("DELETE FROM memories WHERE id=?", (db_id,))
                cursor.execute("DELETE FROM memories_fts WHERE content_id=?", (db_id,))
            
            # Insert consolidated rows
            for fact in consolidated:
                # Check if it already exists to avoid re-adding
                cursor.execute("SELECT id FROM memories WHERE content=? AND category='user_fact'", (fact,))
                if not cursor.fetchone():
                    insert_memory(conn, fact, "user_fact")
                    
            conn.commit()
            print(f"Memory consolidation complete. Deleted {len(deleted_ids)} old records, inserted consolidated beliefs.")
    except Exception as e:
        print(f"Error during memory consolidation: {e}")

if __name__ == "__main__":
    process_dream()
