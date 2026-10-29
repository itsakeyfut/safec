void fill(int ***pp, int *v) { **pp = v; }
int main(void) {
    int x = 0;
    int *m[2];
    int **e = m + 1;
    fill(&e, &x);
    int *r = m[0];
    return r == 0;
}
