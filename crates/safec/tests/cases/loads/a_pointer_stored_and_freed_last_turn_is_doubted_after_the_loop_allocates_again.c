void *malloc(int n);
void free(void *p);

int main(void) {
    int **t2 = malloc(8);
    if (t2 == 0) {
        return 0;
    }
    int i = 0;
    int k = 0;
    while (k < 2) {
        int *p = malloc(4);
        if (p == 0) {
            return 0;
        }
        p[0] = 1;
        if (k == 1) {
            int *q = *t2;
            if (q == 0) {
                return 0;
            }
            return *q;
        }
        *t2 = p;
        free(p);
        k = k + 1;
    }
    return 0;
}
