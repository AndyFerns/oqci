// S;S is Z, not the identity: H Z H = X, so the outcome is 1.
OPENQASM 3.0; include "stdgates.inc";
qubit[1] q; bit[1] c;
h q[0]; s q[0]; s q[0]; h q[0];
c[0] = measure q[0];
