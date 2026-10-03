void *malloc(int n);
void release_all(void);

int f(int n) {
    int **s;
    int **t;
    int *a;
    int i;
    a = malloc(4);
    if (a == 0) {
        return 0;
    }
    a[0] = 1;
    s = malloc(8);
    if (s == 0) {
        return 0;
    }
    t = s;
    i = 0;
    while (i < n) {
        t = s;
        s = malloc(8);
        if (s == 0) {
            return 0;
        }
        i = i + 1;
    }
    *t = a;
    release_all();
    return a[0];
}
