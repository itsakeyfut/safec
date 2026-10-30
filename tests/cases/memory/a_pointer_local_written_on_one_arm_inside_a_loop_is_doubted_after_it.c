int c(void);
int main(void) {
    int *q;
    int **pq = &q;
    int x = 1;
    for (int i = 0; i < 2; i++) {
        if (c()) {
            q = &x;
        }
    }
    int *r = *pq;
    return r == 0;
}
