// A CX between the ends of a line forces routing SWAPs.
OPENQASM 3.0; include "stdgates.inc";
qubit[3] q; bit[3] c;
x q[0]; h q[1]; cx q[0], q[2]; cx q[1], q[0];
c = measure q;
