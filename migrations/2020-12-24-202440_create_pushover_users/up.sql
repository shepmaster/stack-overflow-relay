CREATE TABLE pushover_users (
  key TEXT PRIMARY KEY,
  account_id INTEGER NOT NULL REFERENCES registrations (account_id) UNIQUE
);
