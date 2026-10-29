void *malloc(int n);
void free(void *p);
int main(int c) {
    int x = 0;
    int *m[2];
    int **e = m + 1;
    int ***slot = malloc(8);
    if (slot == 0) {
        return 0;
    }
    *slot = m;
    int ***q = slot;
    if (c) {
        q = &e;
    }
    **q = &x;
    int *r = m[0];
    free(slot);
    return r == 0;
}
