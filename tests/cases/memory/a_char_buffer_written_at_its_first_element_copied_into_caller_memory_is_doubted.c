void *memcpy(void *d, void *s, int n);
void f(int **out) { if (!out) return; char b[8]; b[0] = 0; memcpy(out, b, 8); }
