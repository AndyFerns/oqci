// H X H = Z leaves |0>; reordered to X H H = X it would give |1>.
OPENQASM 3.0; include "stdgates.inc";
qubit[1] q; bit[1] c;
h q[0]; x q[0]; h q[0];
c[0] = measure q[0];
