const int g = 1;
int main(void) {
    *(int *)&g = 2;
    return g;
}
void through_a_parameter(const int *p) {
    if (!p) {
        return;
    }
    *(int *)p = 2;
}
const int *get(void) {
    return &g;
}
int through_a_const_result(void) {
    int *q = (int *)get();
    if (!q) {
        return 1;
    }
    *q = 2;
    return 0;
}
int only_read(const int *p) {
    if (!p) {
        return 0;
    }
    return *p;
}
const int *gp;
void through_a_global(void) {
    int *q = (int *)gp;
    if (!q) {
        return;
    }
    *q = 1;
}
