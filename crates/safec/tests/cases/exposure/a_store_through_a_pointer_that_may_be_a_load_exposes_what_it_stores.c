void *malloc(int n);
void release_all(void);

int f(int c, int ***tab) {
    int **s;
    int **t;
    int *a;
    if (tab == 0) {
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
    if (c) {
        t = s;
    } else {
        t = *tab;
    }
    if (t == 0) {
        return 0;
    }
    *t = a;
    release_all();
    return a[0];
}
