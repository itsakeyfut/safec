void release(void);
int f(int **pp) {
    if (pp == 0) {
        return 0;
    }
    int *r = *pp;
    int **pr = &r;
    int *s = *pr;
    if (s == 0) {
        return 0;
    }
    release();
    return *s;
}
