void *memcpy(void *d, void *s, int n);
int f(int c, int n, int ***pp) {
    int y = 1;
    int *s = &y;
    int *a;
    int **q = &a;
    int ***t = &q;
    memcpy(*t, &s, 8);
    return *a;
}
