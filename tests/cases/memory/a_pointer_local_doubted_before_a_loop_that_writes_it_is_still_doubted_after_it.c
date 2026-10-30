int c(void);
int d(void);
int main(void) {
    int x = 1;
    int *q;
    int **pq = &q;
    if (c()) {
        q = &x;
    }
    while (d()) {
        q = &x;
    }
    int *r = *pq;
    return r == 0;
}
