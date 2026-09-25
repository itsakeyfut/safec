void *malloc(int n);
int f(void) { int *p = malloc(8); int i; i = p * 1; int j; j = p & 1; return i + j; }
