void get(int **pp);

int main(void) {
    int *q;
    int **pq = &q;
    get(pq);
    int *r = *pq;
    return r == 0;
}
