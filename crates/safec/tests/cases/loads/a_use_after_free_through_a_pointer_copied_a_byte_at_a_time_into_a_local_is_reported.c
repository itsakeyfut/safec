void *malloc(int n);
void release(int *q);

int main(void) {
    int **tab = malloc(8);
    if (tab == 0) {
        return 0;
    }
    int *slot = 0;
    int *p = malloc(4);
    if (p == 0) {
        return 0;
    }
    *tab = p;
    void *vd = &slot;
    void *vs = tab;
    char *d = vd;
    char *s = vs;
    int i = 0;
    while (i < 8) {
        if (d != 0) {
            if (s != 0) {
                *d = *s;
            }
        }
        d = d + 1;
        s = s + 1;
        i = i + 1;
    }
    release(slot);
    return *p;
}
