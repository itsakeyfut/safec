int past_it(int n) {
    goto in;
    typedef char T[n];
in:;
    T a;
    return (int)sizeof a;
}

int past_an_alias(int n) {
    goto in;
    typedef char T[n];
    {
        typedef T U;
    }
in:;
    return (int)sizeof(T);
}
