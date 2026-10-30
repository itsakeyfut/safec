int c(void);
int main(void) {
    int x = 1;
    int *q;
    int **pq = &q;
    if (c()) {
        q = &x;
    }
    int *r = *pq;
    return r == 0;
}
