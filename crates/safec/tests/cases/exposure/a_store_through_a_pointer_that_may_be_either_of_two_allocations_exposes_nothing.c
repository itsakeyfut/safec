void *malloc(int n);
void release_all(void);

int f(int c) {
    int **s1 = malloc(8);
    int **s2 = malloc(8);
    int **t;
    int *a = malloc(4);
    if (s1 == 0) {
        return 0;
    }
    if (s2 == 0) {
        return 0;
    }
    if (a == 0) {
        return 0;
    }
    a[0] = 1;
    if (c) {
        t = s1;
    } else {
        t = s2;
    }
    *t = a;
    release_all();
    return a[0];
}
