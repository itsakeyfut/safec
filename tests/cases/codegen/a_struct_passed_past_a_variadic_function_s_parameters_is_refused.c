struct pair {
    long long a;
    long long b;
};

int tail(int a, ...);

int past_the_parameters(struct pair p) {
    return tail(1, p);
}
