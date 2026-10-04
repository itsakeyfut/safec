void release_all(void);

int f(int **pp, int i) {
    if (pp == 0) {
        return 0;
    }
    int *r = *pp;
    int *q = r + i;
    if (q == 0) {
        return 0;
    }
    release_all();
    return *q;
}
