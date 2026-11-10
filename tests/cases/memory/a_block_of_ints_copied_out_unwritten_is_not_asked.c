void *malloc(int n);
void *memcpy(void *d, void *s, int n);
void copied(int ** _Nonnull out) {
    int *x = malloc(4);
    memcpy(out, &x, 8);
}
