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

Add `?time_zone=Asia/Tokyo` to a GraphQL scenario URL to require that `Time-Zone`
header on every GraphQL and async merge request for that scenario.

## Async merge scenarios

Set `GH_CHK_API_BASE_URL` to `http://127.0.0.1:18080/rest/<scenario>` and
`GH_CHK_GRAPHQL_URL` to `http://127.0.0.1:18080/graphql/<scenario>`.
The stub accepts `PUT /repos/owner1/repo1/pulls/1/merge-async` and polls at
`GET /repos/owner1/repo1/pulls/1/merge-async/<uuid>`. It verifies the API version,
media type, authorization header, merge options, and returned request ID.

- `merge_pending`, `merge_timezone`: accepted, pending, then merged.
- `merge_conflict`: an existing request (HTTP 409), pending, then merged.
- `merge_enqueued`: pending, then added to the merge queue.
- `merge_already_merged`, `merge_already_enqueued`: immediate completion.
- `merge_failed`, `merge_rejected`: asynchronous or immediate failure.
- `merge_forbidden`, `merge_conflict_error`, `merge_poll_error`: HTTP errors.
- `merge_invalid`: malformed accepted response.
- `merge_blocked`: a PR that must be skipped by bulk merge.

The old GraphQL merge mutation is rejected. CLI tests override both API URLs,
so merge requests stay local.

## Default test behavior

If `GH_CHK_TEST_STUB_BASE_URL` is not set, `tests/cli.rs` starts the same stub locally with Python and targets it automatically.
