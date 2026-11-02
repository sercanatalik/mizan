# EVDS fixtures

- `no_key.json`: the real body EVDS3 returns with HTTP 403 when the `key`
  header is missing (captured 2026-11-02 from
  `https://evds3.tcmb.gov.tr/igmevdsms-dis/series=TP.FG.J0&startDate=01-01-2025&endDate=01-06-2025&type=json`).
- `yiufe.json`: synthesized in the documented response shape (`items`, `Tarih`
  as `YYYY-M`, the series code with dots replaced by underscores, `null` for a
  month not yet published). The values are illustrative, not official figures.
  Replace it with a trimmed live response once one is captured with a key.
