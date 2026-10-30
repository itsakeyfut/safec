int c(void);
int main(void) {
    int x = 1;
    int *q;
    int **pq = &q;
    if (c()) {
        *pq = &x;
    }
    return q == 0;
}
