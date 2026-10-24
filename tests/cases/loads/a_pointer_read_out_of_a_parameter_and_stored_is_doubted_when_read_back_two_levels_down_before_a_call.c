void *malloc(int n);
void release_all(void);

int f(int **pp, int i) {
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
    int ***bb = malloc(8);
    if (bb == 0) {
        return 0;
    }
    *box = q;
    *bb = box;
    int *r = **bb;
    if (r == 0) {
        return 0;
    }
    release_all();
    return *r;
}
