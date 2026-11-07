int f(int x) {
    int *m[1];
    int **box[1];
    box[0] = m;
    int **t = box[0];
    *t = &x;
    int *q = m[0];
    return *q;
}
