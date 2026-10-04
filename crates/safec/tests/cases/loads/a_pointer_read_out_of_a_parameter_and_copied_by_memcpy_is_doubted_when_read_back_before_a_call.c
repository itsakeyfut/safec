void *malloc(int n);
void *memcpy(void *d, void *s, int n);
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
    int **box2 = malloc(8);
    if (box2 == 0) {
        return 0;
    }
    *box = q;
    memcpy(box2, box, 8);
    int *r = *box2;
    if (r == 0) {
        return 0;
    }
    release_all();
    return *r;
}
