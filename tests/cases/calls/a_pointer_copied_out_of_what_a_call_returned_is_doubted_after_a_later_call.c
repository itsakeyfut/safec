void *malloc(int n);
void *memcpy(void *d, void *s, int n);
void release(void);
int **get_slot(void);
int f(void) {
    int **pp = get_slot();
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
    release();
    return *r;
}
