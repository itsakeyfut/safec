void *malloc(int n);
int cond(void);
void release_all(void);

int f(int ***pp) {
    if (pp == 0) {
        return 0;
    }
    int ***t3 = malloc(8);
    if (t3 == 0) {
        return 0;
    }
    int **t2 = malloc(8);
    if (t2 == 0) {
        return 0;
    }
    *t3 = t2;
    if (cond()) {
        *t3 = *pp;
    }
    int *r = malloc(4);
    if (r == 0) {
        return 0;
    }
    r[0] = 1;
    **t3 = r;
    release_all();
    return *r;
}
