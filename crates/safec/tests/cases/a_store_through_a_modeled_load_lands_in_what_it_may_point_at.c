void *malloc(int n);
void keep(int **p);
void release_all(void);

int f(int c) {
    int ***box = malloc(8);
    int **s = malloc(8);
    int **s2 = malloc(8);
    int **t;
    int *a = malloc(4);
    if (box == 0) {
        return 0;
    }
    if (s == 0) {
        return 0;
    }
    if (s2 == 0) {
        return 0;
    }
    if (a == 0) {
        return 0;
    }
    a[0] = 1;
    *box = s;
    if (c) {
        t = s2;
    } else {
        t = *box;
    }
    *t = a;
    keep(s);
    release_all();
    return a[0];
}
