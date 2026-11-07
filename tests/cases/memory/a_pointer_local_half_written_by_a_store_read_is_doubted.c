int f(void) { int *p; void *v = &p; int *ip = v; *ip = 0; int *r = p; return r == 0; }
