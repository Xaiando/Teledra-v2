import os
import sys
import json
import sqlite3

DB_PATH = "knowledge/memory.db"

def main():
    if len(sys.argv) < 2:
        # Return empty list if no query provided
        print(json.dumps([]))
        sys.exit(0)

    query = sys.argv[1].strip()
    if not query:
        print(json.dumps([]))
        return

    if not os.path.exists(DB_PATH):
        print(json.dumps([]))
        return

    conn = sqlite3.connect(DB_PATH)
    cursor = conn.cursor()

    try:
        # Check if table exists
        cursor.execute("SELECT count(*) FROM sqlite_master WHERE type='table' AND name='memories'")
        if cursor.fetchone()[0] == 0:
            print(json.dumps([]))
            conn.close()
            return
            
        # Clean query terms for FTS5 syntax
        clean_words = []
        for word in query.split():
            # Strip non-alphanumeric chars
            cleaned = "".join(char for char in word if char.isalnum())
            if cleaned:
                clean_words.append(cleaned)
                
        if not clean_words:
            # Fallback: get recent memories if no valid search words
            cursor.execute("SELECT content FROM memories ORDER BY timestamp DESC LIMIT 3")
        else:
            # Join words with OR for broad matching, sorting by BM25 rank
            fts_query = " OR ".join(clean_words)
            cursor.execute("""
                SELECT m.content FROM memories m
                JOIN memories_fts f ON m.id = f.content_id
                WHERE memories_fts MATCH ?
                ORDER BY rank
                LIMIT 3
            """, (fts_query,))
            
        results = [row[0] for row in cursor.fetchall()]
        print(json.dumps(results))
    except Exception as e:
        # Safe fallback: return empty list on any query errors
        print(json.dumps([]))
    finally:
        conn.close()

if __name__ == "__main__":
    main()
