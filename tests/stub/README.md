# Test stub

This stub serves canned GitHub API responses over HTTP so CLI tests can hit a local server instead of reading fixtures in application code.

## Run with Docker

```sh
docker compose -f tests/stub/docker-compose.yml up --build -d
GH_CHK_TEST_STUB_BASE_URL=http://127.0.0.1:18080/graphql cargo test --locked
```

## Supported GraphQL paths

- `/graphql/prs`
- `/graphql/prs_paginated`
- `/graphql/issues`

## Default test behavior

If `GH_CHK_TEST_STUB_BASE_URL` is not set, `tests/cli.rs` starts the same stub locally with Python and targets it automatically.
