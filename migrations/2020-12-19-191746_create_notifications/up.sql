CREATE TABLE notifications (
  id SERIAL PRIMARY KEY,
  account_id INTEGER NOT NULL REFERENCES registrations (account_id),
  text TEXT NOT NULL,
  created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),

  UNIQUE (account_id, text)
);
