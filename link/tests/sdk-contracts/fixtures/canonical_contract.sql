CREATE SCHEMA school;

CREATE TYPE school.role_kind AS ENUM ('owner', 'teacher', 'parent', 'student');

CREATE DOMAIN school.non_empty_code AS TEXT NOT NULL CHECK (VALUE <> '');

CREATE TYPE school.contact_info AS (
  email TEXT NOT NULL,
  phone TEXT,
  tags TEXT[]
);

CREATE TYPE school.audit_meta AS (
  actor_id TEXT NOT NULL,
  contact school.contact_info,
  labels TEXT[]
);

CREATE TABLE school.enrollments (
  school_id TEXT NOT NULL,
  student_id BIGINT NOT NULL,
  course_code school.non_empty_code,
  seq_no BIGINT GENERATED ALWAYS AS IDENTITY,
  display_code TEXT GENERATED ALWAYS AS (school_id || ':' || course_code) STORED,
  role school.role_kind NOT NULL DEFAULT 'student',
  tuition NUMERIC(12, 2) NOT NULL DEFAULT 0.00,
  enrolled_at TIMESTAMPTZ NOT NULL DEFAULT CURRENT_TIMESTAMP,
  metadata JSONB,
  syllabus FILE,
  audit school.audit_meta,
  PRIMARY KEY (school_id, student_id)
) WITH (TYPE = 'SHARED');

CREATE PROCEDURE school.ping();

CREATE FUNCTION school.book_slot(
  IN lesson_id BIGINT NOT NULL,
  IN student_id BIGINT NOT NULL,
  IN note TEXT DEFAULT 'standard',
  VARIADIC tags TEXT[]
) RETURNS BOOLEAN;

CREATE FUNCTION school.lookup_enrollment(
  IN school_id TEXT NOT NULL,
  IN student_id BIGINT NOT NULL
) RETURNS ROW TYPE school.enrollments;

CREATE FUNCTION school.list_enrollments(
  IN school_id TEXT NOT NULL,
  IN limit_rows INT DEFAULT 50
) RETURNS SETOF school.enrollments;

CREATE FUNCTION school.enrollment_summary(
  IN school_id TEXT NOT NULL
) RETURNS TABLE (
  student_id BIGINT,
  role school.role_kind,
  tuition NUMERIC(12, 2)
);
