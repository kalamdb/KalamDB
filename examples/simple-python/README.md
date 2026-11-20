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
```

`kalam dev` holds the terminal and shuts the server down when it stops, so leave
it running and use a second terminal:

```bash
export KALAMDB_PASSWORD=...   # whatever `kalam dev` printed
python main.py
```

## What you get from the generated code

```python
from generated.kalam import NoteState, Notes, notes

async def example(client):
    sql, params = notes.insert_sql({"author": "manik", "body": "hi",
                                    "state": NoteState.DRAFT})
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

The mapping `insert_sql` takes is a generated `TypedDict` (`NotesInsert`), and
`update_sql` takes `NotesUpdate`, so a type checker sees the same column types
the row class carries:

```python
notes.insert_sql({"author": 42, "body": "hi", "state": NoteState.DRAFT})
#                 ^ error: Incompatible types (expression has type "int",
#                   TypedDict item "author" has type "str")

notes.insert_sql({"body": "hi", "state": NoteState.DRAFT})
#                 ^ error: Missing key "author" for TypedDict "NotesInsert"
```

`id` and `created_at` are optional keys because the server fills them;
`author`, `body` and `state` are required because they are `NOT NULL` with no
default. Values are encoded on the way out, so a `NoteState` member, a
`datetime` or a `date` can be passed as-is.

## Array columns

`notes.tags` is declared `TEXT[]` and this example never writes it. The SQL API
binds scalar parameters only, so an array cannot be sent as `$1` yet — the
server answers `Parameter $N invalid: Array parameters not yet supported`.

The SDK refuses the bind before the statement is sent, so the failure names the
column instead of a placeholder position:

```python
notes.insert_sql({"author": "manik", "body": "hi",
                  "state": NoteState.DRAFT, "tags": ["intro", "python"]})
# kalamdb.schema.codecs.UnsupportedParameterError: simple_python.notes.tags:
# the server does not accept array or object parameters yet ...
```

Reading arrays works: a `TEXT[]` column decodes to `Optional[List[str]]` like
any other column. Until array binding lands on the server, write such a column
with a literal in your own SQL, or leave it out of the write.
