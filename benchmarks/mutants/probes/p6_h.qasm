// A lone H, decomposed by h-to-rz-sx, observed through a CX.
OPENQASM 3.0; include "stdgates.inc";
qubit[2] q; bit[2] c;
h q[0]; cx q[0], q[1]; h q[0];
c = measure q;
