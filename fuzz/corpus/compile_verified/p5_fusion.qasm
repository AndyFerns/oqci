// A fusible run whose relative phase is observed through the CX and final H.
OPENQASM 3.0; include "stdgates.inc";
qubit[2] q; bit[2] c;
h q[0]; t q[0]; h q[0]; s q[0];
cx q[0], q[1]; h q[0];
c = measure q;
