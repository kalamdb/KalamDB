CREATE TABLE IF NOT EXISTS room (
  id           BLOB PRIMARY KEY NOT NULL CHECK(is_uuid_v7(id)) DEFAULT (uuid_v7()),
  name         TEXT
) STRICT;

CREATE TABLE IF NOT EXISTS message (
  id           BLOB PRIMARY KEY NOT NULL CHECK(is_uuid_v7(id)) DEFAULT (uuid_v7()),
  _owner       BLOB NOT NULL,
  room         BLOB NOT NULL,
  data         TEXT NOT NULL DEFAULT 'empty',

  -- on user delete, tombstone it.
  FOREIGN KEY(_owner) REFERENCES _user(id) ON DELETE SET NULL,
  -- On chat `room` delete, delete message
  FOREIGN KEY(room) REFERENCES room(id) ON DELETE CASCADE
) STRICT;

CREATE TABLE IF NOT EXISTS room_members (
  user         BLOB NOT NULL,
  room         BLOB NOT NULL,

  FOREIGN KEY(room) REFERENCES room(id) ON DELETE CASCADE,
  FOREIGN KEY(user) REFERENCES _user(id) ON DELETE CASCADE
) STRICT;


-- Add a room (hard-coded id to simplify setup).
INSERT INTO room (id, name) VALUES (X'0191FC9984C577939EC599F82B5D2308', 'room0');

-- And add 'user@localhost' to the room as a member.
INSERT INTO
  room_members (user, room)
SELECT user_id, room_id FROM
  (SELECT id AS user_id FROM _user WHERE email = 'user@localhost'),
  (SELECT id AS room_id FROM room WHERE name = 'room0');
