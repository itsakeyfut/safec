void *malloc(int n);
void *memcpy(void *d, void *s, int n);
void h(int **pp) { if (pp == 0) { return; } int *p = *pp; if (p == 0) { return; } *p = 1; }
int main(void) { char c = 0; void *v = &c; int *q = v; int **box = malloc(8); if (box == 0) { return 1; } memcpy(box, &q, 8); h(box); return c; }
