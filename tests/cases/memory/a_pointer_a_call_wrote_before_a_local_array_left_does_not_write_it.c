void get(int ***out);
int f(int ***b) {
    if (b == 0) { return 1; }
    int **t = 0;
    get(&t);
    if (t == 0) { return 1; }
    int *m[1];
    *b = m;
    *t = 0;
    int *q = m[0];
    if (q == 0) { return 1; }
    *q = 1;
    return 0;
}
