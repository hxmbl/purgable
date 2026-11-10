# Purgable benchmark summary

- commit: bae3786ca44add392ed0c633cd2514539e139060
- binary_version: purgable v2.3
- os: Linux 6.1.189+
- arch: x86_64
- cpu: Intel(R) Xeon(R) Processor
- logical_cores: 2
- filesystem: ext2/ext3
- rustc: rustc 1.99.1 (58cf8f9c3 2026-10-13)
- cache: warm: fixtures are created immediately before each measurement and the page cache is never dropped
- tree has uncommitted changes: true

Times are medians of valid samples; spread is max-min. Rejected samples are excluded and counted.

## Read-only

| workload | op | jobs | files | dirs | candidates | n | median ms | min | max | spread | rejected |
|---|---|---|---|---|---|---|---|---|---|---|---|
| historical | mark_dry_run | 1 | 32503 | 4099 | 128 | 5 | 23.1 | 21.5 | 25.2 | 3.7 | 0 |
| historical | mark_dry_run | 2 | 32503 | 4099 | 128 | 5 | 15.0 | 14.6 | 15.5 | 0.9 | 0 |
| historical | list | 1 | 32503 | 4099 | 128 | 5 | 25.7 | 25.3 | 29.1 | 3.8 | 0 |
| historical | list | 2 | 32503 | 4099 | 128 | 5 | 19.6 | 18.5 | 20.3 | 1.8 | 0 |
| historical | review_dry_run | 1 | 32503 | 4099 | 128 | 5 | 26.7 | 25.9 | 28.6 | 2.7 | 0 |
| historical | review_dry_run | 2 | 32503 | 4099 | 128 | 5 | 18.4 | 18.1 | 20.4 | 2.4 | 0 |
| broad | mark_dry_run | 1 | 16000 | 2000 | 128 | 5 | 11.4 | 11.4 | 12.5 | 1.1 | 0 |
| broad | mark_dry_run | 2 | 16000 | 2000 | 128 | 5 | 9.4 | 9.2 | 9.6 | 0.3 | 0 |
| broad | list | 1 | 16000 | 2000 | 128 | 5 | 13.6 | 13.5 | 14.2 | 0.6 | 0 |
| broad | list | 2 | 16000 | 2000 | 128 | 5 | 9.6 | 9.4 | 9.9 | 0.5 | 0 |
| broad | review_dry_run | 1 | 16000 | 2000 | 128 | 5 | 14.4 | 14.0 | 16.7 | 2.7 | 0 |
| broad | review_dry_run | 2 | 16000 | 2000 | 128 | 5 | 9.7 | 9.3 | 9.8 | 0.5 | 0 |
| high_fanout | mark_dry_run | 1 | 20000 | 5200 | 200 | 5 | 18.9 | 18.4 | 19.7 | 1.3 | 0 |
| high_fanout | mark_dry_run | 2 | 20000 | 5200 | 200 | 5 | 17.3 | 16.9 | 17.4 | 0.5 | 0 |
| high_fanout | list | 1 | 20000 | 5200 | 200 | 5 | 19.9 | 19.5 | 24.6 | 5.1 | 0 |
| high_fanout | list | 2 | 20000 | 5200 | 200 | 5 | 20.4 | 18.7 | 21.5 | 2.8 | 0 |
| high_fanout | review_dry_run | 1 | 20000 | 5200 | 200 | 5 | 19.1 | 18.5 | 19.4 | 0.9 | 0 |
| high_fanout | review_dry_run | 2 | 20000 | 5200 | 200 | 5 | 19.1 | 18.7 | 19.2 | 0.5 | 0 |
| deep_chain | mark_dry_run | 1 | 2001 | 1000 | 1 | 5 | 12.7 | 12.7 | 13.1 | 0.4 | 0 |
| deep_chain | mark_dry_run | 2 | 2001 | 1000 | 1 | 5 | 21.8 | 21.6 | 22.6 | 1.0 | 0 |
| deep_chain | list | 1 | 2001 | 1000 | 1 | 5 | 20.6 | 20.3 | 20.7 | 0.4 | 0 |
| deep_chain | list | 2 | 2001 | 1000 | 1 | 5 | 34.1 | 33.8 | 34.3 | 0.5 | 0 |
| deep_chain | review_dry_run | 1 | 2001 | 1000 | 1 | 5 | 20.9 | 20.8 | 21.1 | 0.3 | 0 |
| deep_chain | review_dry_run | 2 | 2001 | 1000 | 1 | 5 | 34.2 | 32.5 | 34.6 | 2.2 | 0 |
| file_heavy | mark_dry_run | 1 | 60000 | 200 | 5 | 5 | 20.0 | 19.1 | 32.8 | 13.7 | 0 |
| file_heavy | mark_dry_run | 2 | 60000 | 200 | 5 | 5 | 10.7 | 10.7 | 10.7 | 0.1 | 0 |
| file_heavy | list | 1 | 60000 | 200 | 5 | 5 | 23.4 | 22.8 | 28.2 | 5.4 | 0 |
| file_heavy | list | 2 | 60000 | 200 | 5 | 5 | 12.6 | 12.5 | 12.6 | 0.1 | 0 |
| file_heavy | review_dry_run | 1 | 60000 | 200 | 5 | 5 | 22.7 | 22.5 | 28.4 | 5.9 | 0 |
| file_heavy | review_dry_run | 2 | 60000 | 200 | 5 | 5 | 12.7 | 12.5 | 12.8 | 0.3 | 0 |
| many_small_roots | mark_dry_run | 1 | 18000 | 6000 | 3000 | 5 | 47.7 | 46.5 | 48.3 | 1.7 | 0 |
| many_small_roots | mark_dry_run | 2 | 18000 | 6000 | 3000 | 5 | 38.6 | 37.7 | 39.2 | 1.5 | 0 |
| many_small_roots | list | 1 | 18000 | 6000 | 3000 | 5 | 50.6 | 46.9 | 53.2 | 6.3 | 0 |
| many_small_roots | list | 2 | 18000 | 6000 | 3000 | 5 | 40.8 | 40.5 | 41.3 | 0.8 | 0 |
| many_small_roots | review_dry_run | 1 | 18000 | 6000 | 3000 | 5 | 51.3 | 50.7 | 53.1 | 2.4 | 0 |
| many_small_roots | review_dry_run | 2 | 18000 | 6000 | 3000 | 5 | 40.8 | 39.6 | 41.5 | 1.9 | 0 |
| few_huge_roots | mark_dry_run | 1 | 32004 | 4008 | 4 | 5 | 32.1 | 30.5 | 41.7 | 11.1 | 0 |
| few_huge_roots | mark_dry_run | 2 | 32004 | 4008 | 4 | 5 | 18.8 | 18.5 | 18.9 | 0.4 | 0 |
| few_huge_roots | list | 1 | 32004 | 4008 | 4 | 5 | 47.8 | 47.1 | 55.6 | 8.6 | 0 |
| few_huge_roots | list | 2 | 32004 | 4008 | 4 | 5 | 30.7 | 29.7 | 31.7 | 2.0 | 0 |
| few_huge_roots | review_dry_run | 1 | 32004 | 4008 | 4 | 5 | 48.4 | 46.7 | 56.1 | 9.4 | 0 |
| few_huge_roots | review_dry_run | 2 | 32004 | 4008 | 4 | 5 | 29.7 | 29.3 | 30.5 | 1.2 | 0 |
| nested_markers | mark_dry_run | 1 | 12800 | 1000 | 200 | 5 | 12.7 | 11.9 | 14.6 | 2.7 | 0 |
| nested_markers | mark_dry_run | 2 | 12800 | 1000 | 200 | 5 | 7.8 | 7.7 | 8.0 | 0.2 | 0 |
| nested_markers | list | 1 | 12800 | 1000 | 200 | 5 | 31.4 | 30.7 | 33.7 | 2.9 | 0 |
| nested_markers | list | 2 | 12800 | 1000 | 200 | 5 | 23.5 | 21.0 | 26.6 | 5.6 | 0 |
| nested_markers | review_dry_run | 1 | 12800 | 1000 | 200 | 5 | 31.1 | 30.8 | 32.3 | 1.4 | 0 |
| nested_markers | review_dry_run | 2 | 12800 | 1000 | 200 | 5 | 22.8 | 20.7 | 23.6 | 2.9 | 0 |
| wide_dir | mark_dry_run | 1 | 30001 | 1 | 1 | 5 | 19.6 | 18.2 | 27.5 | 9.3 | 0 |
| wide_dir | mark_dry_run | 2 | 30001 | 1 | 1 | 5 | 18.7 | 18.5 | 18.8 | 0.2 | 0 |
| wide_dir | list | 1 | 30001 | 1 | 1 | 5 | 23.8 | 23.7 | 31.0 | 7.3 | 0 |
| wide_dir | list | 2 | 30001 | 1 | 1 | 5 | 24.1 | 24.0 | 24.3 | 0.3 | 0 |
| wide_dir | review_dry_run | 1 | 30001 | 1 | 1 | 5 | 24.2 | 23.9 | 32.0 | 8.1 | 0 |
| wide_dir | review_dry_run | 2 | 30001 | 1 | 1 | 5 | 24.1 | 24.0 | 24.3 | 0.3 | 0 |

## Ordinary delete

| workload | op | jobs | files removed | dirs | candidates | n | median ms | min | max | spread | files/s | MiB/s | rejected |
|---|---|---|---|---|---|---|---|---|---|---|---|---|---|
| historical | delete | 1 | 8024 | 4099 | 128 | 3 | 130.5 | 129.6 | 144.8 | 15.2 | 61464.9 | 183.1 | 0 |
| historical | delete | 2 | 8024 | 4099 | 128 | 3 | 116.0 | 107.4 | 154.8 | 47.4 | 69167.8 | 206.0 | 0 |
| broad | delete | 1 | 3912 | 2000 | 128 | 3 | 70.6 | 70.1 | 70.8 | 0.7 | 55381.3 | 162.2 | 0 |
| broad | delete | 2 | 3912 | 2000 | 128 | 3 | 62.6 | 62.3 | 62.7 | 0.4 | 62446.5 | 182.9 | 0 |
| high_fanout | delete | 1 | 991 | 5200 | 200 | 3 | 32.5 | 31.9 | 32.7 | 0.7 | 30486.2 | 44.0 | 0 |
| high_fanout | delete | 2 | 991 | 5200 | 200 | 3 | 42.1 | 40.9 | 43.9 | 3.0 | 23553.9 | 34.0 | 0 |
| deep_chain | delete | 1 | 1001 | 1000 | 1 | 3 | 57.6 | 56.4 | 58.7 | 2.3 | 17365.9 | 52.5 | 0 |
| deep_chain | delete | 2 | 1001 | 1000 | 1 | 3 | 76.8 | 74.3 | 77.1 | 2.9 | 13035.3 | 39.4 | 0 |
| file_heavy | delete | 1 | 17452 | 200 | 5 | 3 | 276.2 | 273.8 | 294.0 | 20.2 | 63180.3 | 191.2 | 0 |
| file_heavy | delete | 2 | 17452 | 200 | 5 | 3 | 181.4 | 168.3 | 199.8 | 31.5 | 96213.8 | 291.2 | 0 |
| many_small_roots | delete | 1 | 18000 | 6000 | 3000 | 3 | 250.5 | 249.4 | 257.8 | 8.4 | 71865.3 | 181.3 | 0 |
| many_small_roots | delete | 2 | 18000 | 6000 | 3000 | 3 | 394.5 | 391.4 | 396.4 | 5.0 | 45623.2 | 115.1 | 0 |
| few_huge_roots | delete | 1 | 32004 | 4008 | 4 | 3 | 525.8 | 513.5 | 543.4 | 29.8 | 60863.0 | 184.2 | 0 |
| few_huge_roots | delete | 2 | 32004 | 4008 | 4 | 3 | 304.6 | 302.1 | 329.9 | 27.9 | 105077.9 | 318.1 | 0 |
| nested_markers | delete | 1 | 12800 | 1000 | 200 | 3 | 172.1 | 171.1 | 173.9 | 2.8 | 74361.5 | 211.0 | 0 |
| nested_markers | delete | 2 | 12800 | 1000 | 200 | 3 | 211.0 | 210.8 | 213.3 | 2.5 | 60654.6 | 172.1 | 0 |
| wide_dir | delete | 1 | 30001 | 1 | 1 | 3 | 370.5 | 354.0 | 370.8 | 16.8 | 80976.8 | 245.1 | 0 |
| wide_dir | delete | 2 | 30001 | 1 | 1 | 3 | 364.8 | 351.3 | 366.7 | 15.4 | 82240.3 | 249.0 | 0 |

## Shred (reported separately; fdatasync per file)

| workload | op | jobs | files removed | dirs | candidates | n | median ms | min | max | spread | files/s | MiB/s | rejected |
|---|---|---|---|---|---|---|---|---|---|---|---|---|---|
| historical | shred | 1 | 606 | 410 | 67 | 3 | 202.0 | 189.5 | 203.9 | 14.4 | 3000.7 | 9.1 | 0 |
| historical | shred | 2 | 606 | 410 | 67 | 3 | 206.3 | 205.7 | 215.4 | 9.7 | 2938.0 | 8.9 | 0 |
| broad | shred | 1 | 223 | 200 | 25 | 3 | 76.5 | 74.3 | 84.0 | 9.7 | 2916.9 | 8.9 | 0 |
| broad | shred | 2 | 223 | 200 | 25 | 3 | 76.8 | 76.4 | 78.6 | 2.2 | 2902.5 | 8.8 | 0 |
| high_fanout | shred | 1 | 73 | 4200 | 161 | 3 | 41.1 | 38.2 | 42.5 | 4.3 | 1777.6 | 0.9 | 0 |
| high_fanout | shred | 2 | 73 | 4200 | 161 | 3 | 47.4 | 47.1 | 48.0 | 0.9 | 1541.3 | 0.8 | 0 |
| deep_chain | shred | 1 | 500 | 1000 | 1 | 3 | 239.1 | 226.7 | 253.1 | 26.3 | 2091.2 | 6.3 | 0 |
| deep_chain | shred | 2 | 500 | 1000 | 1 | 3 | 180.1 | 177.6 | 186.0 | 8.5 | 2775.5 | 8.4 | 0 |
| file_heavy | shred | 1 | 1749 | 200 | 5 | 3 | 575.8 | 575.1 | 586.7 | 11.7 | 3037.6 | 9.2 | 0 |
| file_heavy | shred | 2 | 1749 | 200 | 5 | 3 | 349.9 | 349.7 | 350.6 | 0.9 | 4999.1 | 15.1 | 0 |
| many_small_roots | shred | 1 | 1500 | 600 | 300 | 3 | 490.5 | 473.9 | 493.1 | 19.2 | 3057.9 | 9.3 | 0 |
| many_small_roots | shred | 2 | 1500 | 600 | 300 | 3 | 492.7 | 481.2 | 509.1 | 27.8 | 3044.6 | 9.2 | 0 |
| few_huge_roots | shred | 1 | 3200 | 408 | 4 | 3 | 1053.9 | 1036.9 | 1073.8 | 37.0 | 3036.3 | 9.2 | 0 |
| few_huge_roots | shred | 2 | 3200 | 408 | 4 | 3 | 640.6 | 640.5 | 641.3 | 0.7 | 4995.7 | 15.1 | 0 |
| nested_markers | shred | 1 | 1200 | 100 | 20 | 3 | 392.9 | 388.9 | 396.0 | 7.1 | 3054.5 | 9.2 | 0 |
| nested_markers | shred | 2 | 1200 | 100 | 20 | 3 | 243.7 | 241.0 | 245.3 | 4.3 | 4924.1 | 14.9 | 0 |
| wide_dir | shred | 1 | 3000 | 1 | 1 | 3 | 978.6 | 964.9 | 999.4 | 34.5 | 3065.6 | 9.3 | 0 |
| wide_dir | shred | 2 | 3000 | 1 | 1 | 3 | 575.7 | 569.3 | 593.7 | 24.4 | 5210.8 | 15.8 | 0 |

