void *malloc(int n);
void *memcpy(void *d, void *s, int n);
void release_all(void);

int f(int **pp) {
    if (pp == 0) {
        return 0;
    }
    int **box = malloc(8);
    if (box == 0) {
        return 0;
    }
    memcpy(box, pp, 8);
    int *r = *box;
    if (r == 0) {
        return 0;
    }
    release_all();
    return *r;
}
