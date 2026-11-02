US Bureau of Labor Statistics fixtures (public API v1, no key), fetched 2026-11-02.

- `cpi_2017_2026.json`: real response to a POST to
  `https://api.bls.gov/publicAPI/v1/timeseries/data/` with
  `{"seriesid":["CUUR0000SA0"],"startyear":"2017","endyear":"2026"}`.
- `unknown_series.json`: real response for a series that does not exist.
- `quota.json`: written from the documented daily-limit response
  (`REQUEST_NOT_PROCESSED`), because it cannot be triggered on demand.
