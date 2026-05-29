-- Coverage for documented user management SQL syntax.

DROP USER IF EXISTS 'case26_user';
DROP USER IF EXISTS 'case26_service';

CREATE USER 'case26_user'
  WITH PASSWORD 'Case26Pass!123'
  ROLE user
  EMAIL 'case26_user@example.com';

ALTER USER 'case26_user' SET EMAIL 'case26_user_new@example.com';
ALTER USER 'case26_user' SET ROLE service;
ALTER USER 'case26_user' SET ROLE user;
ALTER USER 'case26_user' SET PASSWORD 'Case26Pass!456';

CREATE USER 'case26_service'
  WITH OIDC '{"issuer":"https://example.com","subject":"case26_service"}'
  ROLE service
  EMAIL 'case26_service@example.com';

DROP USER IF EXISTS 'case26_service';
DROP USER IF EXISTS 'case26_user';