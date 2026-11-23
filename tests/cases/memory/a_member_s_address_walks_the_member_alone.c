struct Pair {
    int a;
    int b;
};

int past_the_member(void) {
    struct Pair v;
    v.b = 0;
    int *m = &v.a;
    m[1] = 1;
    return v.b;
}

int before_the_member(void) {
    struct Pair v;
    v.a = 0;
    int *m = &v.b;
    m[-1] = 1;
    return v.a;
}

int inside_the_member(void) {
    struct Pair v;
    int *m = &v.b;
    *m = 1;
    return v.b - 1;
}

int main(void) {
    return past_the_member() + before_the_member() + inside_the_member();
}
