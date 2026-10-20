import sys
from pathlib import Path

# The harness is run from benchmarks/ as `python -m oqci_bench`; make it
# importable the same way under pytest.
sys.path.insert(0, str(Path(__file__).resolve().parents[1]))
