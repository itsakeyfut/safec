void keep(int ***p);
int f(int ****c) {
    if (c == 0) { return 1; }
    int *m[1];
    int **t = 0;
    keep(&t);
    int ***pq = *c;
    if (pq == 0) { return 1; }
    *pq = m;
    *t = 0;
    int *q = m[0];
    if (q == 0) { return 1; }
    *q = 1;
    return 0;
}
