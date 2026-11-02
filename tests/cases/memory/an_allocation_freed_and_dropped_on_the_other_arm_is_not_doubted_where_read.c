void *malloc(int n);
void free(void *p);
int f(int k, int **pp) {
    if (!pp) return 0;
    int **m = malloc(8);
    if (!m) return 0;
    if (k) {
        *m = *pp;
    } else {
        free(m);
        m = 0;
    }
    if (!m) return 0;
    int *r = *m;
    free(m);
    if (!r) return 0;
    return *r;
}
