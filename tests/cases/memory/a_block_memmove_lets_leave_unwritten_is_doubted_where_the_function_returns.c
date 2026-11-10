void *malloc(int n);
void *memmove(void *d, void *s, int n);
void moved(int *** _Nonnull out) {
    int **b = malloc(8);
    memmove(out, &b, 8);
}
