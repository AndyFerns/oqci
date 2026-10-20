"""OQCI's benchmark and research harness.

Implements ``final-deliverables-spec.md`` §20 (benchmark infrastructure),
§21 (plots from serialized data), §22 (reproducibility), §24 (timing) and
§30 (the research package), under the experimental protocol locked in
``docs/core_architecture/stage-g-benchmarking-protocol.md``.

Every numeric parameter — families, sizes, seeds, repetitions, shots,
tolerances — comes from a config file (``benchmarks/configs/``), never from
this code. The only defaults here are structural.

Run ``python -m oqci_bench --help`` from the ``benchmarks/`` directory.
"""

__version__ = "1.0.0"
