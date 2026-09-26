import cudaq

@cudaq.kernel
def bell():
    q = cudaq.qvector(2)
    h(q[0])
    x.ctrl(q[0], q[1])
    mz(q)

if __name__ == "__main__":
    print(cudaq.sample(bell, shots_count=1000))
