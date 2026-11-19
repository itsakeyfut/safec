void *malloc(int n); void *memcpy(void *d, void *s, int n);
void keep(int **p);
int f(void) {
    int *slot = 0;
    keep(&slot);
    char *c = malloc(3);
    if (!c) return 0;
    void *t = &slot;
    void **vo = t;
    memcpy(vo, &c, 8);
    return 1;
}
