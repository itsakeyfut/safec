void *malloc(int n);
void log_line(void);
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
    u = s;
    log_line();
    if (c) {
        t = s;
    } else {
        t = u;
    }
    *t = a;
    release_all();
    return a[0];
}
