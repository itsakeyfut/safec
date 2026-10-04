void *malloc(int n);
void free(void *p);
int cond(void);
void release(int ***t);

int main(void) {
    int *r = malloc(4);
    if (r == 0) {
        return 0;
    }
    r[0] = 1;
    int ***t2 = malloc(8);
    if (t2 == 0) {
        return 0;
    }
    int k = 0;
    while (k < 2) {
        int **p = malloc(8);
        if (p == 0) {
            return 0;
        }
        if (k == 1) {
            release(t2);
            return *r;
        }
        *p = r;
        *t2 = p;
        if (cond()) {
            free(p);
        }
        k = k + 1;
    }
    return 0;
}
