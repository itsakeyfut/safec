void *malloc(int n);
void *realloc(void *p, int n);
void release_all(void);

int f(int **pp) {
    if (pp == 0) {
        return 0;
    }
    int *q = *pp;
    if (q == 0) {
        return 0;
    }
    int **box = malloc(8);
    if (box == 0) {
        return 0;
    }
    *box = q;
    int **big = realloc(box, 16);
    if (big == 0) {
        return 0;
    }
    release_all();
    int *r = *big;
    if (r == 0) {
        return 0;
    }
    return *r;
}
