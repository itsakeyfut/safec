void release_all(void);
int f(int ***ppp) {
    if (ppp == 0) { return 0; }
    int **pp = *ppp;
    if (pp == 0) { return 0; }
    int *q = *pp;
    if (q == 0) { return 0; }
    release_all();
    return *q;
}
