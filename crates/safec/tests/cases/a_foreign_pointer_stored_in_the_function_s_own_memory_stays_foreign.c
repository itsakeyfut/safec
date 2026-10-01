void *malloc(int n);
void release_all(void);

int f(int c, int ***tab) {
    int ***box;
    int **s;
    int **u;
    int **t;
    int *a;
    if (tab == 0) {
        return 0;
    }
    box = malloc(8);
    s = malloc(8);
    a = malloc(4);
    if (box == 0) {
        return 0;
    }
    if (s == 0) {
        return 0;
    }
    if (a == 0) {
        return 0;
    }
    a[0] = 1;
    u = *tab;
    *box = u;
    if (c) {
        t = s;
    } else {
        t = *box;
    }
    if (t == 0) {
        return 0;
    }
    *t = a;
    release_all();
    return a[0];
}
