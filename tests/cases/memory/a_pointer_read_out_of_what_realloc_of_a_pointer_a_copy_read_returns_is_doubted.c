// What a library copy reads out of memory may be null, as a load's
// (#931).
void *realloc(void *p, int n);
void free(void *p);
void *memcpy(void *d, void *s, int n);
int g(int *** _Nonnull q) {
    int **old;
    memcpy(&old, q, 8);
    int **pp = realloc(old, 8);
    if (!pp) return 1;
    int *p = *pp;
    free(pp);
    return p != 0;
}
