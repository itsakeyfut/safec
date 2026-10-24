void free(void *p);

int f(int **pp, int *r) {
    if (pp == 0) {
        return 0;
    }
    int *q = *pp;
    if (q == 0) {
        return 0;
    }
    free(r);
    return *q;
}
