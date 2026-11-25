INSERT INTO configs (key, val, last_updated)
VALUES (
    'bootstrap',
    jsonb_build_object('completed', EXISTS (SELECT 1 FROM users)),
    now()
)
ON CONFLICT (key) DO NOTHING;
