# simple-python

The smallest KalamDB Python project: one table declared in `schema.sql`, typed
bindings generated from it, and a script that inserts and reads rows.

## How the pieces fit

```
kalam/schema.sql       you edit this          the only place tables are defined
generated/kalam.py     kalam schema gen       row class, typed columns, table
main.py                you write this         uses the generated bindings
```

`generated/kalam.py` is committed so you can read it without running anything.
Regenerate it whenever `schema.sql` changes:

```bash
kalam schema gen --languages python
```

The generated file contains no DDL. Creating and altering tables is the job of
`schema.sql` plus `kalam db migrate`, so the client can never disagree with the
server about what a table looks like.

## Run it

```bash
pip install kalamdb
kalam dev                     # starts the server and applies schema.sql

export KALAMDB_PASSWORD=...   # whatever `kalam dev` printed
python main.py
```

## What you get from the generated code

```python
from generated.kalam import NoteState, Notes, notes

sql, params = notes.insert_sql({"author": "manik", "body": "hi",
                                "state": NoteState.DRAFT.value})
await client.query(sql, params)

sql, params = notes.select_sql("author = $1", ["manik"])
note: Notes = notes.decode((await client.query_rows(sql, params))[0])

note.created_at      # datetime, decoded from the wire
note.state           # NoteState enum
note.tags            # Optional[List[str]]
```

Every builder returns the SQL string and its parameters, so you can log them,
adapt them, or fall back to `client.query` for anything the builders do not
cover.
