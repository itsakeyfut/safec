void *malloc(int n);
void get(int ***out);
void release_all(void);

int f(int c) {
    int **s;
    int **u;
    int **t;
    int *a;
    s = malloc(8);
    if (s == 0) {
        return 0;
    }
    a = malloc(4);
    if (a == 0) {
        return 0;
    }
    a[0] = 1;
    get(&u);
    if (c) {
        t = u;
    } else {
        t = s;
    }
    if (t == 0) {
        return 0;
    }
    *t = a;
    release_all();
    return a[0];
}
