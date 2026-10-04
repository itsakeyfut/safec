void *malloc(int n);
void free(void *p);

int main(void) {
    int ***t3 = malloc(8);
    if (t3 == 0) {
        return 0;
    }
    int k = 0;
    while (k < 2) {
        int **t2 = malloc(8);
        if (t2 == 0) {
            return 0;
        }
        int *p = malloc(4);
        if (p == 0) {
            return 0;
        }
        p[0] = 1;
        if (k == 1) {
            int *q = **t3;
            if (q == 0) {
                return 0;
            }
            return *q;
        }
        *t2 = p;
        *t3 = t2;
        free(p);
        k = k + 1;
    }
    return 0;
}
