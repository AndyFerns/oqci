// Two Rz's rotation-merge combines; the SX makes the merged angle observable.
OPENQASM 3.0; include "stdgates.inc";
qubit[1] q; bit[1] c;
h q[0]; rz(0.3) q[0]; rz(0.4) q[0]; sx q[0];
c[0] = measure q[0];
