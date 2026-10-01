void *malloc(int n);
void release_all(void);

int f(int c) {
    int ***s2;
    int **s;
    int **t;
    int *a;
    s2 = malloc(8);
    if (s2 == 0) {
        return 0;
    }
    s = malloc(8);
    if (s == 0) {
        return 0;
    }
    a = malloc(4);
    if (a == 0) {
        return 0;
    }
    a[0] = 1;
    *s2 = s;
    t = *s2;
    if (t == 0) {
        return 0;
    }
    *t = a;
    release_all();
    return a[0];
}
