void *malloc(int n); void *memcpy(void *d, void *s, int n);
void keep(int **p);
int f(int k) {
    int *slot = 0;
    if (k) keep(&slot);
    char *c = malloc(3);
    if (!c) return 0;
    void *t = &slot;
    void **vo = t;
    memcpy(vo, &c, 8);
    return 1;
}
