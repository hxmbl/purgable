# Purgable benchmark summary

- commit: f42612e52efab08a919bfc83a5771eb85e7001b7
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
| historical | mark_dry_run | 1 | 32503 | 4099 | 128 | 5 | 22.9 | 22.2 | 32.7 | 10.5 | 0 |
| historical | mark_dry_run | 2 | 32503 | 4099 | 128 | 5 | 15.8 | 15.6 | 16.1 | 0.4 | 0 |
| historical | list | 1 | 32503 | 4099 | 128 | 5 | 26.8 | 26.2 | 28.3 | 2.0 | 0 |
| historical | list | 2 | 32503 | 4099 | 128 | 5 | 19.1 | 18.9 | 19.8 | 0.8 | 0 |
| historical | review_dry_run | 1 | 32503 | 4099 | 128 | 5 | 27.2 | 26.1 | 29.3 | 3.2 | 0 |
| historical | review_dry_run | 2 | 32503 | 4099 | 128 | 5 | 19.2 | 18.9 | 19.3 | 0.5 | 0 |
| broad | mark_dry_run | 1 | 16000 | 2000 | 128 | 5 | 12.0 | 12.0 | 13.3 | 1.3 | 0 |
| broad | mark_dry_run | 2 | 16000 | 2000 | 128 | 5 | 9.9 | 9.0 | 10.3 | 1.3 | 0 |
| broad | list | 1 | 16000 | 2000 | 128 | 5 | 13.8 | 13.4 | 14.4 | 1.0 | 0 |
| broad | list | 2 | 16000 | 2000 | 128 | 5 | 10.9 | 10.5 | 11.0 | 0.5 | 0 |
| broad | review_dry_run | 1 | 16000 | 2000 | 128 | 5 | 13.7 | 13.4 | 14.3 | 0.9 | 0 |
| broad | review_dry_run | 2 | 16000 | 2000 | 128 | 5 | 10.4 | 10.2 | 10.7 | 0.5 | 0 |
| high_fanout | mark_dry_run | 1 | 20000 | 5200 | 200 | 5 | 19.3 | 19.1 | 20.4 | 1.3 | 0 |
| high_fanout | mark_dry_run | 2 | 20000 | 5200 | 200 | 5 | 17.7 | 17.6 | 18.0 | 0.3 | 0 |
| high_fanout | list | 1 | 20000 | 5200 | 200 | 5 | 19.1 | 18.8 | 19.4 | 0.6 | 0 |
| high_fanout | list | 2 | 20000 | 5200 | 200 | 5 | 19.7 | 19.7 | 20.0 | 0.3 | 0 |
| high_fanout | review_dry_run | 1 | 20000 | 5200 | 200 | 5 | 19.1 | 18.9 | 19.6 | 0.6 | 0 |
| high_fanout | review_dry_run | 2 | 20000 | 5200 | 200 | 5 | 19.6 | 19.5 | 19.7 | 0.3 | 0 |
| deep_chain | mark_dry_run | 1 | 2001 | 1000 | 1 | 5 | 12.8 | 12.8 | 12.9 | 0.1 | 0 |
| deep_chain | mark_dry_run | 2 | 2001 | 1000 | 1 | 5 | 22.4 | 20.3 | 22.8 | 2.5 | 0 |
| deep_chain | list | 1 | 2001 | 1000 | 1 | 5 | 21.3 | 20.8 | 23.2 | 2.4 | 0 |
| deep_chain | list | 2 | 2001 | 1000 | 1 | 5 | 35.2 | 32.9 | 35.4 | 2.4 | 0 |
| deep_chain | review_dry_run | 1 | 2001 | 1000 | 1 | 5 | 21.5 | 21.1 | 29.9 | 8.8 | 0 |
| deep_chain | review_dry_run | 2 | 2001 | 1000 | 1 | 5 | 36.8 | 36.3 | 37.2 | 1.0 | 0 |
| file_heavy | mark_dry_run | 1 | 60000 | 200 | 5 | 5 | 20.5 | 19.5 | 24.6 | 5.1 | 0 |
| file_heavy | mark_dry_run | 2 | 60000 | 200 | 5 | 5 | 11.5 | 11.0 | 12.3 | 1.3 | 0 |
| file_heavy | list | 1 | 60000 | 200 | 5 | 5 | 27.6 | 23.6 | 30.2 | 6.6 | 0 |
| file_heavy | list | 2 | 60000 | 200 | 5 | 5 | 12.9 | 12.7 | 13.5 | 0.8 | 0 |
| file_heavy | review_dry_run | 1 | 60000 | 200 | 5 | 5 | 24.3 | 23.5 | 30.0 | 6.4 | 0 |
| file_heavy | review_dry_run | 2 | 60000 | 200 | 5 | 5 | 13.5 | 13.0 | 13.5 | 0.5 | 0 |
| many_small_roots | mark_dry_run | 1 | 18000 | 6000 | 3000 | 5 | 47.5 | 46.9 | 52.7 | 5.7 | 0 |
| many_small_roots | mark_dry_run | 2 | 18000 | 6000 | 3000 | 5 | 39.0 | 37.8 | 39.5 | 1.7 | 0 |
| many_small_roots | list | 1 | 18000 | 6000 | 3000 | 5 | 50.0 | 49.8 | 50.9 | 1.1 | 0 |
| many_small_roots | list | 2 | 18000 | 6000 | 3000 | 5 | 41.6 | 41.1 | 43.1 | 2.0 | 0 |
| many_small_roots | review_dry_run | 1 | 18000 | 6000 | 3000 | 5 | 51.4 | 50.2 | 52.3 | 2.1 | 0 |
| many_small_roots | review_dry_run | 2 | 18000 | 6000 | 3000 | 5 | 41.6 | 41.4 | 42.8 | 1.3 | 0 |
| few_huge_roots | mark_dry_run | 1 | 32004 | 4008 | 4 | 5 | 33.0 | 30.7 | 42.8 | 12.1 | 0 |
| few_huge_roots | mark_dry_run | 2 | 32004 | 4008 | 4 | 5 | 20.2 | 19.8 | 20.3 | 0.5 | 0 |
| few_huge_roots | list | 1 | 32004 | 4008 | 4 | 5 | 49.6 | 48.0 | 51.9 | 3.9 | 0 |
| few_huge_roots | list | 2 | 32004 | 4008 | 4 | 5 | 31.9 | 31.1 | 32.3 | 1.1 | 0 |
| few_huge_roots | review_dry_run | 1 | 32004 | 4008 | 4 | 5 | 51.0 | 49.1 | 60.0 | 10.9 | 0 |
| few_huge_roots | review_dry_run | 2 | 32004 | 4008 | 4 | 5 | 32.3 | 31.8 | 32.9 | 1.1 | 0 |
| nested_markers | mark_dry_run | 1 | 12800 | 1000 | 200 | 5 | 13.3 | 12.7 | 14.4 | 1.7 | 0 |
| nested_markers | mark_dry_run | 2 | 12800 | 1000 | 200 | 5 | 8.3 | 8.2 | 8.5 | 0.3 | 0 |
| nested_markers | list | 1 | 12800 | 1000 | 200 | 5 | 31.6 | 31.5 | 32.3 | 0.8 | 0 |
| nested_markers | list | 2 | 12800 | 1000 | 200 | 5 | 23.6 | 23.2 | 24.0 | 0.7 | 0 |
| nested_markers | review_dry_run | 1 | 12800 | 1000 | 200 | 5 | 31.5 | 31.4 | 31.7 | 0.3 | 0 |
| nested_markers | review_dry_run | 2 | 12800 | 1000 | 200 | 5 | 23.7 | 23.4 | 24.2 | 0.9 | 0 |
| wide_dir | mark_dry_run | 1 | 30001 | 1 | 1 | 5 | 19.5 | 18.7 | 27.1 | 8.4 | 0 |
| wide_dir | mark_dry_run | 2 | 30001 | 1 | 1 | 5 | 18.9 | 18.8 | 19.2 | 0.4 | 0 |
| wide_dir | list | 1 | 30001 | 1 | 1 | 5 | 26.1 | 24.7 | 31.6 | 6.9 | 0 |
| wide_dir | list | 2 | 30001 | 1 | 1 | 5 | 25.1 | 24.4 | 28.4 | 4.0 | 0 |
| wide_dir | review_dry_run | 1 | 30001 | 1 | 1 | 5 | 24.4 | 23.8 | 30.5 | 6.8 | 0 |
| wide_dir | review_dry_run | 2 | 30001 | 1 | 1 | 5 | 24.6 | 24.3 | 24.7 | 0.4 | 0 |

## Ordinary delete

| workload | op | jobs | files | dirs | candidates | n | median ms | min | max | spread | files/s | MiB/s | rejected |
|---|---|---|---|---|---|---|---|---|---|---|---|---|---|
| historical | delete | 1 | 32503 | 4099 | 128 | 3 | 143.3 | 140.2 | 168.7 | 28.5 | 56013.9 | 684.2 | 0 |
| historical | delete | 2 | 32503 | 4099 | 128 | 3 | 120.6 | 119.6 | 126.1 | 6.5 | 66546.6 | 812.8 | 0 |
| broad | delete | 1 | 16000 | 2000 | 128 | 3 | 64.0 | 62.5 | 68.7 | 6.3 | 61137.0 | 750.9 | 0 |
| broad | delete | 2 | 16000 | 2000 | 128 | 3 | 61.2 | 60.0 | 63.0 | 3.0 | 63939.7 | 785.3 | 0 |
| high_fanout | delete | 1 | 20000 | 5200 | 200 | 3 | 32.9 | 32.3 | 33.4 | 1.1 | 30099.2 | 1820.6 | 0 |
| high_fanout | delete | 2 | 20000 | 5200 | 200 | 3 | 44.0 | 44.0 | 44.2 | 0.2 | 22498.7 | 1360.9 | 0 |
| deep_chain | delete | 1 | 2001 | 1000 | 1 | 3 | 59.8 | 57.2 | 62.7 | 5.5 | 16750.3 | 101.3 | 0 |
| deep_chain | delete | 2 | 2001 | 1000 | 1 | 3 | 79.6 | 79.0 | 82.5 | 3.6 | 12574.7 | 76.1 | 0 |
| file_heavy | delete | 1 | 60000 | 200 | 5 | 3 | 290.1 | 287.7 | 337.8 | 50.1 | 60156.5 | 626.1 | 0 |
| file_heavy | delete | 2 | 60000 | 200 | 5 | 3 | 236.3 | 220.8 | 243.1 | 22.3 | 73857.5 | 768.6 | 0 |
| many_small_roots | delete | 1 | 18000 | 6000 | 3000 | 3 | 253.4 | 250.8 | 256.4 | 5.6 | 71041.6 | 179.2 | 0 |
| many_small_roots | delete | 2 | 18000 | 6000 | 3000 | 3 | 393.6 | 388.7 | 426.2 | 37.5 | 45733.9 | 115.4 | 0 |
| few_huge_roots | delete | 1 | 32004 | 4008 | 4 | 3 | 509.1 | 507.7 | 524.4 | 16.7 | 62858.6 | 190.3 | 0 |
| few_huge_roots | delete | 2 | 32004 | 4008 | 4 | 3 | 360.4 | 355.2 | 366.9 | 11.7 | 88809.9 | 268.8 | 0 |
| nested_markers | delete | 1 | 12800 | 1000 | 200 | 3 | 172.9 | 172.0 | 173.8 | 1.8 | 74032.4 | 210.1 | 0 |
| nested_markers | delete | 2 | 12800 | 1000 | 200 | 3 | 220.5 | 219.3 | 223.6 | 4.2 | 58051.7 | 164.8 | 0 |
| wide_dir | delete | 1 | 30001 | 1 | 1 | 3 | 362.3 | 354.7 | 363.1 | 8.4 | 82796.5 | 250.6 | 0 |
| wide_dir | delete | 2 | 30001 | 1 | 1 | 3 | 361.0 | 356.3 | 388.1 | 31.8 | 83115.7 | 251.6 | 0 |

## Shred (reported separately; fdatasync per file)

| workload | op | jobs | files | dirs | candidates | n | median ms | min | max | spread | files/s | MiB/s | rejected |
|---|---|---|---|---|---|---|---|---|---|---|---|---|---|
| historical | shred | 1 | 3250 | 410 | 67 | 3 | 214.2 | 204.1 | 222.0 | 18.0 | 3142.2 | 45.0 | 0 |
| historical | shred | 2 | 3250 | 410 | 67 | 3 | 221.0 | 219.4 | 226.4 | 7.0 | 3045.7 | 43.6 | 0 |
| broad | shred | 1 | 1600 | 200 | 25 | 3 | 80.0 | 78.0 | 86.4 | 8.4 | 3099.4 | 59.6 | 0 |
| broad | shred | 2 | 1600 | 200 | 25 | 3 | 82.3 | 80.8 | 82.3 | 1.5 | 3014.4 | 58.0 | 0 |
| high_fanout | shred | 1 | 2000 | 4200 | 161 | 3 | 43.3 | 41.3 | 48.0 | 6.7 | 5405.6 | 128.5 | 0 |
| high_fanout | shred | 2 | 2000 | 4200 | 161 | 3 | 48.4 | 48.4 | 48.6 | 0.2 | 4832.5 | 114.9 | 0 |
| deep_chain | shred | 1 | 1001 | 1000 | 1 | 3 | 237.3 | 236.1 | 244.9 | 8.8 | 2111.4 | 12.8 | 0 |
| deep_chain | shred | 2 | 1001 | 1000 | 1 | 3 | 187.8 | 184.1 | 188.8 | 4.6 | 2668.1 | 16.1 | 0 |
| file_heavy | shred | 1 | 6000 | 200 | 5 | 3 | 605.5 | 579.6 | 612.5 | 32.8 | 2896.8 | 30.0 | 0 |
| file_heavy | shred | 2 | 6000 | 200 | 5 | 3 | 368.0 | 365.6 | 371.0 | 5.4 | 4765.9 | 49.3 | 0 |
| many_small_roots | shred | 1 | 1800 | 600 | 300 | 3 | 505.9 | 492.4 | 510.9 | 18.5 | 3558.1 | 9.0 | 0 |
| many_small_roots | shred | 2 | 1800 | 600 | 300 | 3 | 507.0 | 504.7 | 507.1 | 2.4 | 3550.2 | 9.0 | 0 |
| few_huge_roots | shred | 1 | 3204 | 408 | 4 | 3 | 1124.8 | 1119.5 | 1132.0 | 12.5 | 2848.6 | 8.6 | 0 |
| few_huge_roots | shred | 2 | 3204 | 408 | 4 | 3 | 682.4 | 676.1 | 684.7 | 8.6 | 4695.2 | 14.2 | 0 |
| nested_markers | shred | 1 | 1280 | 100 | 20 | 3 | 421.0 | 404.1 | 423.7 | 19.6 | 3040.7 | 8.6 | 0 |
| nested_markers | shred | 2 | 1280 | 100 | 20 | 3 | 259.9 | 255.9 | 263.3 | 7.4 | 4925.5 | 14.0 | 0 |
| wide_dir | shred | 1 | 3001 | 1 | 1 | 3 | 1048.3 | 1033.6 | 1048.8 | 15.3 | 2862.7 | 8.7 | 0 |
| wide_dir | shred | 2 | 3001 | 1 | 1 | 3 | 617.0 | 615.5 | 617.6 | 2.1 | 4863.9 | 14.7 | 0 |

