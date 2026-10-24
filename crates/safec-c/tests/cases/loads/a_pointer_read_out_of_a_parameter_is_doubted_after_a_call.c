void release_all(void);

int f(int **pp) {
    int *q;
    if (pp == 0) {
        return 0;
    }
    q = *pp;
    if (q == 0) {
        return 0;
    }
    release_all();
    return *q;
}
