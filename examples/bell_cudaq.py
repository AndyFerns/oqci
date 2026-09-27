# The Bell pair as a CUDA-Q kernel — the `cudaq-adapter/` prototype's own
# example, kept when that package was folded into OQCI. Measures the whole
# register with one broadcast `mz(q)`; compare ghz3_cudaq.py, which measures
# qubit by qubit.
#
#   oqci compile examples/bell_cudaq.py
#   oqci prepare examples/bell_cudaq.py --backend simulator-nisq -o bell.json
#   python -m oqci.backends.cudaq run bell.json      # needs CUDA-Q installed
#
# Only the @cudaq.kernel function is compiled; the host code below runs
# unchanged under CUDA-Q itself.
import cudaq


@cudaq.kernel
def bell():
    q = cudaq.qvector(2)
    h(q[0])
    x.ctrl(q[0], q[1])
    mz(q)


if __name__ == "__main__":
    print(cudaq.sample(bell, shots_count=1000))
