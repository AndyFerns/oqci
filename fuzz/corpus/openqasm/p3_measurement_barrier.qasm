// X; measure; X must not cancel across the measurement: c0 reads 1.
OPENQASM 3.0; include "stdgates.inc";
qubit[1] q; bit[2] c;
x q[0]; c[0] = measure q[0]; x q[0];
c[1] = measure q[0];
