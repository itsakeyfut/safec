// A pointer copied by `memcpy`, then freed through the original and used
// through the copy: the use after free is reported where the copy is read,
// `SC0402` at `*q = 1`, as a store `q = p;` would be (#969, #979).
void *malloc(int n);
void free(void *p);
void *memcpy(void *d, void *s, int n);
void f(void) {
    int *p = malloc(4);
    if (!p) return;
    int *q;
    memcpy(&q, &p, 8);
    free(p);
    *q = 1;
}
