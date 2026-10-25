void h(int **pp) { if (pp == 0) { return; } int *p = *pp; if (p == 0) { return; } *p = 1; }
int main(void) { char c = 0; void *v = &c; int *q = v; h(&q); return c; }
