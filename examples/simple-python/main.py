"""Minimal KalamDB Python example.

Tables are declared once in kalam/schema.sql. `kalam schema gen --languages
python` writes generated/kalam.py, which gives you a typed row class, typed
columns, and a table declaration. Nothing here creates tables: that is what
schema.sql and migrations are for.
"""

import asyncio
import os

from kalamdb import Auth, KalamClient

from generated.kalam import NoteState, Notes, notes

URL = os.environ.get("KALAMDB_URL", "http://localhost:2900")
USER = os.environ.get("KALAMDB_USER", "admin")
PASSWORD = os.environ.get("KALAMDB_PASSWORD", "")


async def main() -> None:
    async with KalamClient(URL, Auth.basic(USER, PASSWORD)) as client:
        # INSERT: id and created_at are server-filled, so they are left out.
        sql, params = notes.insert_sql(
            {
                "author": "manik",
                "body": "schema.sql is the source of truth",
                "state": NoteState.PUBLISHED.value,
                "tags": ["intro", "python"],
            }
        )
        await client.query(sql, params)

        # SELECT: the where fragment is parameterised, never interpolated.
        sql, params = notes.select_sql("author = $1", ["manik"])
        rows = await client.query_rows(sql, params)

        for row in rows:
            note: Notes = notes.decode(row)
            # Typed attributes: created_at is a datetime, state is an enum.
            print(f"{note.created_at:%Y-%m-%d}  {note.state.value:9}  {note.body}")
            print(f"  tags={note.tags}")


if __name__ == "__main__":
    asyncio.run(main())
