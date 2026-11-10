# Purgable benchmark summary

- commit: 928031c26c6715c77a9ce7dd2b5c30e51ac45ab5
- binary_version: purgable v2.3
- os: Darwin 25.6.0
- arch: x86_64
- cpu: Intel(R) Core(TM) i5-10500 CPU @ 3.10GHz
- logical_cores: 12
- filesystem: apfs
- rustc: rustc 1.99.1 (58cf8f9c3 2026-10-13) (Homebrew)
- cache: warm: fixtures are created immediately before each measurement and the page cache is never dropped
- tree has uncommitted changes: false

Times are medians of valid samples; spread is max-min. Rejected samples are excluded and counted.

## Read-only

| workload | op | jobs | files | dirs | candidates | n | median ms | min | max | spread | rejected |
|---|---|---|---|---|---|---|---|---|---|---|---|
| historical | mark_dry_run | 1 | 32503 | 4099 | 128 | 5 | 139.3 | 137.7 | 139.7 | 2.0 | 0 |
| historical | mark_dry_run | 2 | 32503 | 4099 | 128 | 5 | 79.7 | 79.5 | 80.1 | 0.6 | 0 |
| historical | mark_dry_run | 4 | 32503 | 4099 | 128 | 5 | 47.4 | 47.0 | 47.7 | 0.6 | 0 |
| historical | mark_dry_run | 8 | 32503 | 4099 | 128 | 5 | 34.4 | 34.2 | 34.9 | 0.7 | 0 |
| historical | mark_dry_run | 12 | 32503 | 4099 | 128 | 5 | 31.1 | 30.6 | 31.2 | 0.6 | 0 |
| historical | list | 1 | 32503 | 4099 | 128 | 5 | 193.8 | 172.7 | 220.6 | 47.9 | 0 |
| historical | list | 2 | 32503 | 4099 | 128 | 5 | 98.7 | 97.1 | 100.0 | 3.0 | 0 |
| historical | list | 4 | 32503 | 4099 | 128 | 5 | 58.6 | 58.3 | 58.9 | 0.7 | 0 |
| historical | list | 8 | 32503 | 4099 | 128 | 5 | 43.2 | 43.1 | 43.7 | 0.6 | 0 |
| historical | list | 12 | 32503 | 4099 | 128 | 5 | 38.7 | 38.3 | 39.2 | 0.9 | 0 |
| historical | review_dry_run | 1 | 32503 | 4099 | 128 | 5 | 167.7 | 166.9 | 168.5 | 1.6 | 0 |
| historical | review_dry_run | 2 | 32503 | 4099 | 128 | 5 | 96.6 | 96.2 | 97.7 | 1.4 | 0 |
| historical | review_dry_run | 4 | 32503 | 4099 | 128 | 5 | 57.3 | 57.0 | 57.7 | 0.7 | 0 |
| historical | review_dry_run | 8 | 32503 | 4099 | 128 | 5 | 41.8 | 41.2 | 42.1 | 0.9 | 0 |
| historical | review_dry_run | 12 | 32503 | 4099 | 128 | 5 | 37.2 | 36.8 | 37.5 | 0.7 | 0 |

## Ordinary delete

| workload | op | jobs | files | dirs | candidates | n | median ms | min | max | spread | files/s | MiB/s | rejected |
|---|---|---|---|---|---|---|---|---|---|---|---|---|---|
| historical | delete | 1 | 32503 | 4099 | 128 | 3 | 971.8 | 937.5 | 998.2 | 60.8 | 8256.9 | 100.9 | 0 |
| historical | delete | 2 | 32503 | 4099 | 128 | 3 | 718.2 | 709.1 | 728.0 | 18.9 | 11172.1 | 136.5 | 0 |
| historical | delete | 4 | 32503 | 4099 | 128 | 3 | 572.1 | 556.1 | 579.8 | 23.7 | 14025.3 | 171.3 | 0 |
| historical | delete | 8 | 32503 | 4099 | 128 | 3 | 560.7 | 529.8 | 570.2 | 40.4 | 14311.8 | 174.8 | 0 |
| historical | delete | 12 | 32503 | 4099 | 128 | 3 | 567.0 | 558.4 | 569.9 | 11.6 | 14151.3 | 172.9 | 0 |

## Shred (reported separately; fdatasync per file)

| workload | op | jobs | files | dirs | candidates | n | median ms | min | max | spread | files/s | MiB/s | rejected |
|---|---|---|---|---|---|---|---|---|---|---|---|---|---|
| historical | shred | 1 | 3250 | 410 | 67 | 3 | 11122.2 | 11099.8 | 11167.2 | 67.4 | 60.5 | 0.9 | 0 |
| historical | shred | 2 | 3250 | 410 | 67 | 3 | 11006.0 | 10947.3 | 11105.1 | 157.8 | 61.1 | 0.9 | 0 |
| historical | shred | 4 | 3250 | 410 | 67 | 3 | 11027.1 | 10998.7 | 11058.6 | 59.9 | 61.0 | 0.9 | 0 |
| historical | shred | 8 | 3250 | 410 | 67 | 3 | 11021.0 | 11001.2 | 11131.5 | 130.3 | 61.1 | 0.9 | 0 |
| historical | shred | 12 | 3250 | 410 | 67 | 3 | 11116.0 | 10882.9 | 11251.3 | 368.3 | 60.5 | 0.9 | 0 |
