void fill(int **s);
int main(void) {
    int *m[3];
    fill(&m[1]);
    fill(m);
    int *r = m[0];
    return r == 0;
}
