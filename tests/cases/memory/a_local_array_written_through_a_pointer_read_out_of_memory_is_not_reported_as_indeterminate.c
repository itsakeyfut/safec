int main(void) {
    int x = 0;
    int *m[1];
    int **h[1];
    h[0] = m;
    int **t = h[0];
    if (t == 0) {
        return 1;
    }
    *t = &x;
    int *q = m[0];
    return q == 0;
}
