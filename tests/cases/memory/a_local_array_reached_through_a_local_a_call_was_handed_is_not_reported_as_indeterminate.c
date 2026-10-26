void fill(int ***pp);

int main(void) {
    int *m[1];
    int **p = m;
    fill(&p);
    int *q = m[0];
    return q == 0;
}
