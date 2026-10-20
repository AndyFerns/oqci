<!-- generated from serialized data; do not edit -->
**Ablation: each configuration minus oqci-default, paired by item**

| config | target | items | mean_delta_2q | mean_delta_gates | mean_delta_depth | items_worse_in_gates |
|---|---|---|---|---|---|---|
| oqci-no-canonicalize | linear | 64 | 0 | 0 | 0 | 0 |
| oqci-no-canonicalize | heron | 64 | 0 | 0 | 0 | 0 |
| oqci-no-gate-cancellation | linear | 64 | 4.484 | 5.594 | 3.469 | 18 |
| oqci-no-gate-cancellation | heron | 64 | -0.4375 | -2.328 | -0.4375 | 15 |
| oqci-no-passes | linear | 64 | 4.484 | 29.88 | 6.797 | 44 |
| oqci-no-passes | heron | 64 | -0.4375 | 21.95 | 2.062 | 39 |
| oqci-no-rotation-merge | linear | 64 | 0 | 0.09375 | 0.04688 | 6 |
| oqci-no-rotation-merge | heron | 64 | 0 | 0.09375 | 0.01562 | 6 |
| oqci-no-schedule | linear | 64 | 0 | 0 | 0 | 0 |
| oqci-no-schedule | heron | 64 | 0 | 0 | 0 | 0 |
| oqci-no-single-qubit-fusion | linear | 64 | 0 | 17.89 | 2.344 | 44 |
| oqci-no-single-qubit-fusion | heron | 64 | 0 | 17.89 | 1.938 | 44 |
| oqci-trivial-layout | linear | 64 | 2.719 | 2.719 | 2.125 | 21 |
| oqci-trivial-layout | heron | 64 | 82.55 | 577.8 | 75.11 | 40 |
