# A three-qubit GHZ state as a CUDA-Q kernel — the same circuit as
# ghz3.qasm minus its redundant gate pair, read by OQCI's CUDA-Q frontend:
#
#   oqci compile examples/ghz3_cudaq.py
#   oqci lower examples/ghz3_cudaq.py --backend simulator-nisq
#
# Only the @cudaq.kernel function is compiled. Everything else in this file
# is host code and is never parsed, so it runs unchanged under CUDA-Q itself.
import cudaq


@cudaq.kernel
def ghz3():
    q = cudaq.qvector(3)
    h(q[0])
    x.ctrl(q[0], q[1])
    x.ctrl(q[1], q[2])
    # One measurement per qubit, each to its own classical bit.
    mz(q[0])
    mz(q[1])
    mz(q[2])


if __name__ == "__main__":
    print(cudaq.sample(ghz3, shots_count=1000))
