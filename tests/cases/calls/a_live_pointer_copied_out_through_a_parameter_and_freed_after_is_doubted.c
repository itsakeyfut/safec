void *malloc(int n);
void free(void *p);
void *memcpy(void *s1, void *s2, int n);
void put(int ** _Nonnull pp) {
    int *x = malloc(4);
    if (x == 0) {
        return;
    }
    memcpy(pp, &x, 8);
    free(x);
}
