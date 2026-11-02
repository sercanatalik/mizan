Twelve Data fixtures.

- `aapl_5d.json`: real response for AAPL with `adjust=none`, fetched with
  Twelve Data's documented `demo` key (which only serves AAPL).
- `error_key.json`: real response to an invalid key.
- `error_limit.json`, `error_symbol.json`: written from the documented error
  shape (`code`, `message`, `status`), because they cannot be triggered
  without a personal key.
