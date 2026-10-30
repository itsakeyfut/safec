int main(int c) {
    int x = 1;
    int *m[3];
    m[2] = &x;
    int **f = m;
    if (c) {
        f = f + 1;
    }
    int *r = *f;
    return r == 0;
}
