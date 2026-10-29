int main(void) {
    int x = 1;
    int *m[16];
    void *v = &m[0] + 8;
    char *c = v;
    c = c - 8;
    void *v2 = c;
    int **w = v2;
    *w = &x;
    int *r = m[0];
    return r == 0;
}
