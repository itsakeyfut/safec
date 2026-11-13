int f(int x) { int *big[100]; big[0] = &x; int *p = big[0]; return p == 0; }
